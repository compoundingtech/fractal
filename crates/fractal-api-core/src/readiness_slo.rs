//! Operator-bound API readiness from a root-owned local producer.
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::ReadinessSloConfig;
use crate::store::Store;

const OBJECTIVE: &str = "fractal-api-readiness";
const MINUTE_MS: u64 = 60_000;
const MINUTES: u64 = 7 * 24 * 60;
const MAX_AGE_MS: u64 = 3 * MINUTE_MS;
const MAX_BYTES: u64 = 1024;
const RETAINED_MAX_BYTES: u64 = 8 * 1024;
const SHORT_MINUTES: u64 = 5;
const MAX_RETAINED_PAIRS: usize = 4;
const EVIDENCE_DIR: &str = "/run/fractal-slo-evidence";
static POLICY: OnceLock<Policy> = OnceLock::new();

struct Policy {
    config: ReadinessSloConfig,
    checkpoint: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Reading {
    version: u64,
    objective: String,
    member: String,
    start_ms: u64,
    end_ms: u64,
    observed_at_ms: u64,
    expected: u64,
    known: u64,
    good: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedPairs {
    version: u64,
    pairs: Vec<RetainedPair>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedPair {
    weekly: Reading,
    short: ShortReading,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShortReading {
    version: u64,
    objective: String,
    member: String,
    end_ms: u64,
    observed_at_ms: u64,
    observations: Vec<ShortObservation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShortObservation {
    at_ms: u64,
    ready: bool,
}

struct ShortSummary {
    end_ms: u64,
    known: usize,
    failed: usize,
}

impl ShortReading {
    fn summary(&self, weekly: &Reading) -> Result<ShortSummary> {
        ensure!(
            self.version == 1
                && self.objective == weekly.objective
                && self.member == weekly.member
                && self.end_ms == weekly.end_ms
                && self.observed_at_ms == weekly.observed_at_ms
                && self.observations.len() <= SHORT_MINUTES as usize,
            "short and weekly evidence must match"
        );
        let start = self
            .end_ms
            .checked_sub(SHORT_MINUTES * MINUTE_MS)
            .context("short evidence has no complete window")?;
        let mut previous = None;
        let mut failed = 0;
        for observation in &self.observations {
            let minute = observation.at_ms / MINUTE_MS;
            ensure!(
                (start..self.end_ms).contains(&observation.at_ms)
                    && previous.is_none_or(|prior| minute > prior),
                "short evidence has invalid or repeated minute"
            );
            previous = Some(minute);
            failed += usize::from(!observation.ready);
        }
        Ok(ShortSummary {
            end_ms: self.end_ms,
            known: self.observations.len(),
            failed,
        })
    }
}

impl RetainedPairs {
    fn latest_at(&self, anchor: &Reading, now: u64) -> Result<Option<ShortSummary>> {
        Ok(self.fresh_through(anchor, now)?.pop())
    }

    fn fresh_through(&self, anchor: &Reading, now: u64) -> Result<Vec<ShortSummary>> {
        ensure!(
            self.version == 1 && self.pairs.len() <= MAX_RETAINED_PAIRS,
            "invalid retained pair bundle"
        );
        let mut previous = None;
        let mut fresh = Vec::new();
        for pair in &self.pairs {
            let weekly = &pair.weekly;
            // Check old entries structurally without letting them age out the entire bundle.
            weekly.check(&anchor.member, weekly.observed_at_ms)?;
            ensure!(
                weekly.observed_at_ms <= now,
                "retained cut is from the future"
            );
            ensure!(
                previous.is_none_or(|end| weekly.end_ms > end),
                "retained cuts must be strictly ordered"
            );
            previous = Some(weekly.end_ms);
            let summary = pair.short.summary(weekly)?;
            if weekly.end_ms == anchor.end_ms {
                ensure!(
                    weekly == anchor,
                    "retained cut conflicts with the weekly source"
                );
            }
            if weekly.end_ms <= anchor.end_ms && now - weekly.end_ms <= MAX_AGE_MS {
                fresh.push(summary);
            }
        }
        if let (Some(first), Some(last)) = (self.pairs.first(), previous) {
            ensure!(
                last - first.weekly.end_ms <= MAX_AGE_MS,
                "retained cuts span too much time"
            );
        }
        Ok(fresh)
    }
}

fn short_row(member: &str, reading: Option<ShortSummary>) -> Value {
    let status = match &reading {
        Some(reading) if reading.known >= 4 && reading.failed >= 3 => "missed",
        Some(reading) if reading.known >= 4 => "met",
        _ => "no-data",
    };
    json!({
        "id": format!("{OBJECTIVE}-early/{member}"),
        "kind": "objective",
        "about": "API readiness early-warning evidence",
        "target": "Fewer than 3 failed probe minutes with at least 4 completed",
        "current": reading.map_or_else(|| "No trusted paired short reading".to_owned(), |reading| {
            format!("{} failed of {} completed probe minutes at cut {}", reading.failed, reading.known, reading.end_ms)
        }),
        "window": "5m",
        "remaining_percent": null,
        "status": status,
        "explain": "The most recent trusted pair at or before the weekly cut. Fewer than four completed minutes is unknown. This row is observational.",
    })
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Checkpoint {
    Cut { reading: Reading, pending: bool },
    Legacy(Reading),
}

impl Checkpoint {
    fn reading(&self) -> &Reading {
        match self {
            Self::Cut { reading, .. } | Self::Legacy(reading) => reading,
        }
    }
}

/// Install the single operator-selected policy when the daemon starts.
pub fn configure(
    config: Option<&ReadinessSloConfig>,
    state_dir: &Path,
    origin: &str,
) -> Result<()> {
    let Some(config) = config.filter(|config| config.executor == origin) else {
        return Ok(());
    };
    ensure!(
        evidence_path_valid(&config.evidence_file),
        "readiness_slo.evidence_file must name a file directly under {EVIDENCE_DIR}"
    );
    ensure!(
        config
            .retained_pairs_file
            .as_deref()
            .is_none_or(|path| { path != config.evidence_file && evidence_path_valid(path) }),
        "readiness_slo.retained_pairs_file must name a different file directly under {EVIDENCE_DIR}"
    );
    ensure!(
        !config.member.is_empty()
            && config.member.len() <= 64
            && config
                .member
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)),
        "readiness_slo.member must be a bounded member identifier"
    );
    ensure!(
        config.reviewer.strip_prefix("agent/").is_some_and(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id.split('/').all(|part| {
                    !part.is_empty()
                        && part != "."
                        && part != ".."
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                })
        }),
        "readiness_slo.reviewer must name an agent"
    );
    ensure!(
        !config.shared_case || config.retained_pairs_file.is_some(),
        "shared readiness cases require retained_pairs_file"
    );
    if let Some(backup) = &config.backup {
        ensure!(
            backup != &config.reviewer
                && backup.strip_prefix("agent/").is_some_and(|id| {
                    !id.is_empty() && id.len() <= 128 && id.split('/').all(|part| {
                        !part.is_empty() && part != "." && part != ".."
                            && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                    })
                }),
            "readiness_slo.backup must name a distinct agent"
        );
    }
    POLICY
        .set(Policy {
            config: config.clone(),
            checkpoint: state_dir.join("readiness-slo-cut.json"),
        })
        .map_err(|_| anyhow::anyhow!("readiness SLO policy already configured"))
}

fn evidence_path_valid(path: &Path) -> bool {
    path.parent() == Some(Path::new(EVIDENCE_DIR))
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

fn read_json<T: serde::de::DeserializeOwned>(reader: impl std::io::Read) -> Result<T> {
    read_json_bounded(reader, MAX_BYTES)
}

fn read_json_bounded<T: serde::de::DeserializeOwned>(
    reader: impl std::io::Read,
    limit: u64,
) -> Result<T> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "readiness evidence is too large"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn trusted_evidence(path: &Path) -> Result<Reading> {
    read_json(trusted_file(path)?)
}

fn trusted_file(path: &Path) -> Result<File> {
    // Root-owned, non-writable ancestors prevent directory replacement by the daemon user.
    for parent in path.ancestors().skip(1) {
        let meta = fs::symlink_metadata(parent)?;
        ensure!(
            meta.is_dir() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
            "readiness evidence directory is not root-controlled"
        );
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
        "readiness evidence is not a root-controlled regular file"
    );
    Ok(file)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

impl Reading {
    fn check(&self, member: &str, now: u64) -> Result<()> {
        ensure!(
            self.version == 1 && self.objective == OBJECTIVE && self.member == member,
            "readiness evidence has the wrong version, objective or member"
        );
        ensure!(
            self.expected == MINUTES && self.good <= self.known && self.known <= self.expected,
            "invalid readiness counts"
        );
        ensure!(
            self.end_ms.checked_sub(self.start_ms) == Some(MINUTES * MINUTE_MS)
                && self.end_ms.is_multiple_of(MINUTE_MS)
                && self.end_ms <= self.observed_at_ms
                && self.observed_at_ms <= now
                && now - self.end_ms <= MAX_AGE_MS,
            "invalid or stale readiness window"
        );
        Ok(())
    }

    fn status(&self) -> &'static str {
        let target = self.expected * 999;
        if self.good * 1000 >= target {
            "met"
        } else if (self.good + self.expected - self.known) * 1000 < target {
            "missed"
        } else {
            "no-data"
        }
    }

    fn follows(&self, prior: &Self) -> bool {
        self.member == prior.member
            && self.objective == prior.objective
            && (self.end_ms > prior.end_ms || self == prior)
    }
}

impl Policy {
    fn checked_reading(&self, now: u64) -> Result<(Reading, bool)> {
        let reading = trusted_evidence(&self.config.evidence_file)?;
        let changed = self.admit(&reading, now)?;
        Ok((reading, changed))
    }

    fn admit(&self, reading: &Reading, now: u64) -> Result<bool> {
        self.validate_after(reading, now, self.read_checkpoint()?.as_ref())
    }

    fn validate_after(
        &self,
        reading: &Reading,
        now: u64,
        checkpoint: Option<&Checkpoint>,
    ) -> Result<bool> {
        reading.check(&self.config.member, now)?;
        if let Some(checkpoint) = checkpoint {
            let prior = checkpoint.reading();
            ensure!(
                reading.follows(prior),
                "replayed or conflicting readiness evidence"
            );
            return Ok(reading != prior);
        }
        Ok(true)
    }

    fn read_checkpoint(&self) -> Result<Option<Checkpoint>> {
        match File::open(&self.checkpoint) {
            Ok(file) => read_json(file)
                .map(Some)
                .context("readiness evidence checkpoint is invalid"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn current_rows(
        &self,
        store: &Store,
        weekly: Option<&Reading>,
        retained: Option<&RetainedPairs>,
        now: u64,
    ) -> Vec<Value> {
        let mut rows = vec![self.with_binding(store, self.row(weekly)).0];
        if self.config.retained_pairs_file.is_some() {
            let short = weekly.and_then(|weekly| retained?.latest_at(weekly, now).ok().flatten());
            rows.push(short_row(&self.config.member, short));
        }
        rows
    }

    fn row(&self, reading: Option<&Reading>) -> Value {
        json!({
            "id": format!("{OBJECTIVE}/{}", self.config.member),
            "kind": "objective",
            "about": "API readiness from a trusted independent probe",
            "target": "≥ 99.9%",
            "current": reading.map_or_else(|| "No trusted fresh reading".to_owned(), |r| {
                format!("{} good of {} known / {} expected minutes at cut {}", r.good, r.known, r.expected, r.end_ms)
            }),
            "window": "7d",
            "remaining_percent": reading.filter(|r| r.status() != "no-data").map(|r| {
                let remaining = (r.good as f64 / r.expected as f64 - 0.999) / 0.001;
                (remaining * 100.0).clamp(0.0, 100.0)
            }),
            "status": reading.map_or("no-data", Reading::status),
            "explain": "Met if the lower bound reaches 99.9%; missed if the upper bound is below 99.9%. Unknown minutes are neither successes nor failures.",
        })
    }

    fn with_binding(&self, store: &Store, mut row: Value) -> (Value, Option<String>) {
        let issue = match store.fault_reviewer_is_declared(&self.config.reviewer) {
            Ok(true) => return (row, None),
            Ok(false) => format!(
                "configured reviewer {} is not an active declaration",
                self.config.reviewer
            ),
            Err(error) => format!(
                "cannot verify the configured reviewer {}: {error:#}",
                self.config.reviewer
            ),
        };
        let explanation = row["explain"].as_str().unwrap_or_default();
        row["explain"] = json!(format!(
            "{explanation} The {issue}. Correct the deployment binding before fault delivery."
        ));
        (row, Some(issue))
    }

    fn checkpoint(&self, reading: &Reading, pending: bool) -> Result<()> {
        let parent = self
            .checkpoint
            .parent()
            .context("readiness checkpoint has no parent")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(
            &mut temporary,
            &Checkpoint::Cut {
                reading: reading.clone(),
                pending,
            },
        )?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.checkpoint)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    }

    fn reconcile(
        &self,
        store: &Store,
        now: u64,
        next: impl FnOnce() -> Result<Reading>,
    ) -> Result<Value> {
        let checkpoint = self.read_checkpoint()?;
        if let Some(Checkpoint::Cut {
            reading,
            pending: true,
        }) = checkpoint.as_ref()
        {
            // Replay the single previously admitted cut even if the publisher advanced or
            // its evidence is now stale. Native fault transitions deduplicate this retry.
            reading.check(&self.config.member, reading.observed_at_ms)?;
            self.route(store, reading)?;
            self.checkpoint(reading, false)?;
        }
        let reading = next()?;
        let changed = self.validate_after(&reading, now, checkpoint.as_ref())?;
        self.apply(store, &reading, changed)
    }

    fn apply(&self, store: &Store, reading: &Reading, changed: bool) -> Result<Value> {
        if changed {
            self.checkpoint(reading, true)?;
        }
        let row = self.route(store, reading)?;
        if changed {
            self.checkpoint(reading, false)?;
        }
        Ok(row)
    }

    fn route(&self, store: &Store, reading: &Reading) -> Result<Value> {
        let row = self.row(Some(reading));
        let objectives = BTreeSet::from([format!("{OBJECTIVE}/{}", self.config.member)]);
        crate::objective_faults::check_with_response(
            store,
            std::slice::from_ref(&row),
            &objectives,
            crate::objective_faults::ObjectiveResponse {
                reviewer: &self.config.reviewer,
                condition: crate::store::API_READINESS_SLO_MISSED,
                title: "API readiness",
            },
        )
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        Ok(row)
    }
}

/// Read the current objectives without issuing claims or changing the evidence checkpoint.
pub fn current(store: &Store) -> Vec<Value> {
    let Some(policy) = POLICY.get() else {
        return Vec::new();
    };
    let now = now_ms();
    let reading = policy.checked_reading(now).ok().map(|(reading, _)| reading);
    let retained = policy.config.retained_pairs_file.as_ref().and_then(|path| {
        let file = trusted_file(path).ok()?;
        read_json_bounded::<RetainedPairs>(file, RETAINED_MAX_BYTES).ok()
    });
    policy.current_rows(store, reading.as_ref(), retained.as_ref(), now)
}

/// Evaluate and route one source reading on the existing SLO history cadence.
pub fn sample(store: &Store) -> Option<Value> {
    static FAILING: AtomicBool = AtomicBool::new(false);
    static INVALID_BINDING: AtomicBool = AtomicBool::new(false);
    let policy = POLICY.get()?;
    let result = if policy.config.shared_case {
        retained_admission::run_sampler(policy, store, now_ms())
    } else {
        policy.reconcile(store, now_ms(), || trusted_evidence(&policy.config.evidence_file))
    };
    let row = match result {
        Ok(row) => {
            FAILING.store(false, Ordering::Relaxed);
            row
        }
        Err(error) => {
            if !FAILING.swap(true, Ordering::Relaxed) {
                eprintln!("fractal: readiness SLO has no admitted reading: {error:#}");
            }
            policy.row(None)
        }
    };
    let (row, issue) = policy.with_binding(store, row);
    if let Some(issue) = issue {
        if !INVALID_BINDING.swap(true, Ordering::Relaxed) {
            eprintln!("fractal: readiness SLO owner binding is invalid: {issue}");
        }
    } else {
        INVALID_BINDING.store(false, Ordering::Relaxed);
    }
    Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RetirementsStore;
    use crate::model::IntentInput;

    fn reading(minute: u64, known: u64, good: u64) -> Reading {
        let end_ms = (MINUTES + minute) * MINUTE_MS;
        Reading {
            version: 1,
            objective: OBJECTIVE.into(),
            member: "node-sample-a".into(),
            start_ms: end_ms - MINUTES * MINUTE_MS,
            end_ms,
            observed_at_ms: end_ms,
            expected: MINUTES,
            known,
            good,
        }
    }

    fn retained_pair(weekly: &Reading, failures: u64) -> Value {
        json!({
            "weekly": weekly,
            "short": {
                "version": 1,
                "objective": weekly.objective,
                "member": weekly.member,
                "end_ms": weekly.end_ms,
                "observed_at_ms": weekly.observed_at_ms,
                "observations": (1..=4).rev().map(|offset| json!({
                    "at_ms": weekly.end_ms - offset * MINUTE_MS + 1,
                    "ready": offset > failures,
                })).collect::<Vec<_>>()
            }
        })
    }

    #[test]
    fn latest_at_preserves_main_reader_result_for_the_same_bundle() {
        let anchor = reading(10, MINUTES, MINUTES);
        let bundle: RetainedPairs = serde_json::from_value(json!({
            "version": 1,
            "pairs": (8..=11).map(|minute| {
                retained_pair(&reading(minute, MINUTES, MINUTES), minute % 4)
            }).collect::<Vec<_>>(),
        })).unwrap();
        for minute in [11, 12, 14] {
            let now = reading(minute, MINUTES, MINUTES).end_ms;
            // Main's reader replaces the latest summary for each eligible pair.
            let mut main_latest = None;
            for pair in &bundle.pairs {
                let summary = pair.short.summary(&pair.weekly).unwrap();
                if pair.weekly.end_ms <= anchor.end_ms
                    && now - pair.weekly.end_ms <= MAX_AGE_MS
                {
                    main_latest = Some(summary);
                }
            }
            let values = |summary: ShortSummary| (summary.end_ms, summary.known, summary.failed);
            assert_eq!(
                bundle.latest_at(&anchor, now).unwrap().map(values),
                main_latest.map(values),
            );
        }
    }

    #[test]
    fn usage_observes_last_fresh_pair_without_writing_the_weekly_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("graph.db"), "node").unwrap();
        let weekly = reading(8, MINUTES, MINUTES);
        let earlier = reading(7, MINUTES, MINUTES - 3);
        let bundle: RetainedPairs = serde_json::from_value(json!({
            "version": 1, "pairs": [retained_pair(&earlier, 3)]
        }))
        .unwrap();
        let mut policy = Policy {
            config: ReadinessSloConfig {
                executor: "node".into(),
                member: weekly.member.clone(),
                reviewer: "agent/node.ops".into(),
                evidence_file: dir.path().join("weekly.json"),
                retained_pairs_file: None,
                shared_case: false,
                backup: None,
            },
            checkpoint: dir.path().join("checkpoint.json"),
        };
        policy.checkpoint(&weekly, false).unwrap();
        let before = fs::read(&policy.checkpoint).unwrap();
        let index = store.index().unwrap();
        let legacy = policy.current_rows(&store, Some(&weekly), Some(&bundle), weekly.end_ms);
        assert_eq!(legacy.len(), 1);
        policy.config.retained_pairs_file = Some(dir.path().join("retained.json"));
        let rows = policy.current_rows(&store, Some(&weekly), Some(&bundle), weekly.end_ms);
        assert_eq!(rows[0], legacy[0]);
        assert_eq!(rows[1]["status"], "missed");
        assert!(
            rows[1]["current"]
                .as_str()
                .unwrap()
                .ends_with(&earlier.end_ms.to_string())
        );
        assert_eq!(
            policy.current_rows(&store, Some(&weekly), None, weekly.end_ms)[1]["status"],
            "no-data"
        );
        assert_eq!(
            policy.current_rows(&store, None, Some(&bundle), weekly.end_ms)[1]["status"],
            "no-data"
        );
        assert_eq!(fs::read(&policy.checkpoint).unwrap(), before);
        assert_eq!(store.index().unwrap(), index);
    }

    #[test]
    fn retained_usage_rejects_conflicts_ordering_and_oversize() {
        let weekly = reading(8, MINUTES, MINUTES);
        let earlier = reading(7, MINUTES, MINUTES - 3);
        let old = reading(5, MINUTES, MINUTES - 3);
        let parse = |pairs| -> RetainedPairs {
            serde_json::from_value(json!({"version": 1, "pairs": pairs})).unwrap()
        };
        let boundary = parse(vec![retained_pair(&old, 3)]);
        assert!(
            boundary
                .latest_at(&weekly, weekly.end_ms)
                .unwrap()
                .is_some()
        );
        assert!(
            boundary
                .latest_at(&weekly, weekly.end_ms + 1)
                .unwrap()
                .is_none()
        );
        let reversed = parse(vec![retained_pair(&weekly, 2), retained_pair(&earlier, 3)]);
        assert!(reversed.latest_at(&weekly, weekly.end_ms).is_err());
        let conflicting = parse(vec![retained_pair(&reading(8, MINUTES, MINUTES - 1), 3)]);
        assert!(conflicting.latest_at(&weekly, weekly.end_ms).is_err());
        let mut wrong_short = retained_pair(&weekly, 3);
        wrong_short["short"]["observed_at_ms"] = json!(weekly.observed_at_ms - 1);
        assert!(
            parse(vec![wrong_short])
                .latest_at(&weekly, weekly.end_ms)
                .is_err()
        );
        let oversized = vec![b' '; RETAINED_MAX_BYTES as usize + 1];
        assert!(
            read_json_bounded::<RetainedPairs>(oversized.as_slice(), RETAINED_MAX_BYTES).is_err()
        );
    }

    #[test]
    fn retained_usage_rejects_untrusted_parent_and_config_paths() {
        let mut config: ReadinessSloConfig = serde_json::from_value(json!({
            "executor": "node", "member": "node-sample-a", "reviewer": "agent/node.ops",
            "evidence_file": "/run/fractal-slo-evidence/weekly.json",
        }))
        .unwrap();
        assert!(config.retained_pairs_file.is_none());
        assert!(!config.shared_case);
        assert!(config.backup.is_none());
        assert!(
            serde_json::to_value(&config)
                .unwrap()
                .get("retained_pairs_file")
                .is_none()
        );
        let dir = tempfile::tempdir().unwrap();
        let unsafe_dir = dir.path().join("mutable");
        fs::create_dir(&unsafe_dir).unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&unsafe_dir, fs::Permissions::from_mode(0o777)).unwrap();
        let path = unsafe_dir.join("retained.json");
        fs::write(&path, "{}").unwrap();
        assert!(trusted_file(&path).is_err());
        config.retained_pairs_file = Some(config.evidence_file.clone());
        assert!(configure(Some(&config), dir.path(), "node").is_err());
        assert!(evidence_path_valid(Path::new(
            "/run/fractal-slo-evidence/retained.json"
        )));
        assert!(!evidence_path_valid(Path::new(
            "/run/fractal-slo-evidence/sub/retained.json"
        )));
        assert!(!evidence_path_valid(Path::new(
            "/run/fractal-slo-evidence/../retained.json"
        )));
    }

    #[test]
    fn counts_can_prove_a_verdict_even_when_minutes_are_unknown() {
        assert_eq!(reading(0, MINUTES - 10, MINUTES - 10).status(), "met");
        assert_eq!(reading(0, MINUTES - 11, MINUTES - 11).status(), "no-data");
        assert_eq!(reading(0, MINUTES - 5, MINUTES - 16).status(), "missed");
        assert_eq!(reading(0, 0, 0).status(), "no-data");
    }

    #[test]
    fn stale_future_wrong_member_and_invalid_counts_are_rejected() {
        let evidence = reading(1, MINUTES, MINUTES);
        assert!(evidence.check("node-sample-a", evidence.end_ms).is_ok());
        assert!(
            evidence
                .check("node-sample-a", evidence.end_ms + MAX_AGE_MS + 1)
                .is_err()
        );
        assert!(
            evidence
                .check("node-sample-a", evidence.end_ms - 1)
                .is_err()
        );
        assert!(evidence.check("node-sample-b", evidence.end_ms).is_err());
        assert!(
            reading(1, 0, 1)
                .check("node-sample-a", evidence.end_ms)
                .is_err()
        );
        let payload = vec![b' '; MAX_BYTES as usize + 1];
        assert!(read_json::<Reading>(payload.as_slice()).is_err());
    }

    #[test]
    fn pending_miss_and_recovery_replay_before_a_newer_cut_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("graph.db");
        let policy = Policy {
            config: ReadinessSloConfig {
                executor: "node".into(),
                member: "node-sample-a".into(),
                reviewer: "agent/node.ops".into(),
                evidence_file: dir.path().join("unused.json"),
                retained_pairs_file: None,
                shared_case: false,
                backup: None,
            },
            checkpoint: dir.path().join("readiness-slo-cut.json"),
        };
        let store = Store::open(&db, "node").unwrap();
        let source = "version 2\nagent \"node.ops\" { command \"true\" }\n";
        let intent = crate::graph::parse_intent(source, "node").unwrap();
        let preview = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store
            .apply(
                &intent,
                &preview.subject_tokens,
                "readiness-pending-reviewer",
            )
            .unwrap();
        let id = "slo-objective/node/fractal-api-readiness/node-sample-a";

        // Crash after writing a pending miss but before appending the fault.
        let missed = reading(1, MINUTES, MINUTES - 11);
        policy.checkpoint(&missed, true).unwrap();
        assert!(store.operational_failure(id).unwrap().is_none());
        drop(store);

        // The pending cut is already stale, and the next fresh cut is inconclusive.
        let store = Store::open(&db, "node").unwrap();
        let successor = reading(5, 0, 0);
        assert_eq!(
            policy
                .reconcile(&store, successor.end_ms, || Ok(successor.clone()))
                .unwrap()["status"],
            "no-data"
        );
        let first = store.operational_failure(id).unwrap().unwrap();
        assert_eq!(first.status, "pending");
        assert_eq!(first.reviewer, "agent/node.ops");

        // Crash before recording recovery. A later miss must open a new episode,
        // rather than being absorbed into the still-open old failure.
        let met = reading(6, MINUTES, MINUTES);
        policy.checkpoint(&met, true).unwrap();
        drop(store);

        let store = Store::open(&db, "node").unwrap();
        let next_miss = reading(7, MINUTES, MINUTES - 11);
        assert_eq!(
            policy
                .reconcile(&store, next_miss.end_ms, || Ok(next_miss.clone()))
                .unwrap()["status"],
            "missed"
        );
        let reopened = store.operational_failure(id).unwrap().unwrap();
        assert_eq!(reopened.status, "pending");
        assert_ne!(reopened.request, first.request);
        assert!(matches!(
            policy.read_checkpoint().unwrap(),
            Some(Checkpoint::Cut { pending: false, .. })
        ));

        // Also tolerate a crash after native routing but before marking committed.
        policy.checkpoint(&next_miss, true).unwrap();
        let index = store.index().unwrap();
        drop(store);
        let store = Store::open(&db, "node").unwrap();
        policy
            .reconcile(&store, next_miss.end_ms, || Ok(next_miss.clone()))
            .unwrap();
        assert_eq!(store.index().unwrap(), index);
        assert_eq!(
            store.operational_failure(id).unwrap().unwrap().request,
            reopened.request
        );
    }

    #[test]
    fn native_fault_uses_the_explicit_reviewer_and_survives_retry_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("graph.db");
        let policy = Policy {
            config: ReadinessSloConfig {
                executor: "node".into(),
                member: "node-sample-a".into(),
                reviewer: "agent/node.ops".into(),
                evidence_file: dir.path().join("unused.json"),
                retained_pairs_file: None,
                shared_case: false,
                backup: None,
            },
            checkpoint: dir.path().join("readiness-slo-cut.json"),
        };
        let store = Store::open(&db, "node").unwrap();
        let source = "version 2\nagent \"node.ops\" { command \"true\" }\nagent \"node.backup\" { command \"true\"; handles-faults }\n";
        let intent = crate::graph::parse_intent(source, "node").unwrap();
        let preview = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        let applied = store
            .apply(&intent, &preview.subject_tokens, "readiness-slo-reviewer")
            .unwrap();
        assert!(applied.changed, "test reviewer was not published");
        assert_eq!(
            store.fleet_fault_agent().unwrap().as_deref(),
            Some("agent/node.backup")
        );

        assert_eq!(
            store
                .selected_desired_kind("agent/node.ops")
                .unwrap()
                .as_deref(),
            Some("agent")
        );
        let id = format!(
            "slo-objective/{}/fractal-api-readiness/node-sample-a",
            store.origin()
        );
        let missed = reading(1, MINUTES, MINUTES - 11);
        let changed = policy.admit(&missed, missed.end_ms).unwrap();
        assert_eq!(
            policy.apply(&store, &missed, changed).unwrap()["status"],
            "missed"
        );
        let first = store.operational_failure(&id).unwrap().unwrap();
        assert_eq!(
            (first.status.as_str(), first.reviewer.as_str()),
            ("pending", "agent/node.ops")
        );
        let fault_owner = |store: &Store| {
            store
                .fault_snapshot(u128::from(now_ms()) + 1)
                .unwrap()
                .into_iter()
                .find(|fault| fault.item.episode == first.request)
                .expect("readiness fault is visible")
                .owner
        };
        assert_eq!(fault_owner(&store).as_deref(), Some("agent/node.ops"));

        let stop_source = "version 2\nstop \"agent/node.ops\"\n";
        let stop = crate::graph::parse_intent(stop_source, "node").unwrap();
        let preview = store
            .mission(
                &stop,
                IntentInput {
                    kdl: stop_source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        store
            .apply(
                &stop,
                &preview.subject_tokens,
                "readiness-slo-stop-reviewer",
            )
            .unwrap();
        assert!(!store.fault_reviewer_is_declared("agent/node.ops").unwrap());
        assert!(
            fault_owner(&store).is_none(),
            "no generic fallback after stop"
        );
        assert_eq!(
            store.operational_failure(&id).unwrap().unwrap().reviewer,
            "agent/node.ops"
        );
        let reviewer = "agent/node.ops";
        let token = store.selected_desired_token(reviewer).unwrap().unwrap();
        store
            .retire_agent(
                reviewer,
                reviewer,
                "fixture completed",
                &token,
                None,
                "readiness-slo-retire-reviewer",
                &[],
            )
            .unwrap();
        assert!(!store.fault_reviewer_is_declared(reviewer).unwrap());
        assert!(
            fault_owner(&store).is_none(),
            "no generic fallback after retirement"
        );
        drop(store);

        let store = Store::open(&db, "node").unwrap();
        let changed = policy.admit(&missed, missed.end_ms).unwrap();
        assert!(!changed);
        policy.apply(&store, &missed, changed).unwrap();
        assert_eq!(
            store.operational_failure(&id).unwrap().unwrap().request,
            first.request
        );
        let unknown = reading(2, 0, 0);
        let changed = policy.admit(&unknown, unknown.end_ms).unwrap();
        assert_eq!(
            policy.apply(&store, &unknown, changed).unwrap()["status"],
            "no-data"
        );
        assert_eq!(
            store.operational_failure(&id).unwrap().unwrap().status,
            "pending"
        );

        let met = reading(3, MINUTES, MINUTES);
        assert!(policy.admit(&missed, met.end_ms).is_err());
        assert!(
            policy
                .admit(&reading(2, MINUTES, MINUTES), met.end_ms)
                .is_err()
        );
        let changed = policy.admit(&met, met.end_ms).unwrap();
        assert_eq!(
            policy.apply(&store, &met, changed).unwrap()["status"],
            "met"
        );
        assert_eq!(
            store.operational_failure(&id).unwrap().unwrap().status,
            "resolved"
        );
        assert!(policy.admit(&missed, met.end_ms).is_err());

        let new_miss = reading(4, MINUTES, MINUTES - 11);
        let changed = policy.admit(&new_miss, new_miss.end_ms).unwrap();
        let row = policy.apply(&store, &new_miss, changed).unwrap();
        let (row, issue) = policy.with_binding(&store, row);
        assert_eq!(row["status"], "missed");
        assert!(issue.unwrap().contains("agent/node.ops"));
        assert!(
            row["explain"]
                .as_str()
                .unwrap()
                .contains("Correct the deployment binding")
        );
        assert_eq!(
            store.operational_failure(&id).unwrap().unwrap().status,
            "resolved"
        );
    }
}

#[path = "readiness_retained_admission.rs"]
mod retained_admission;
