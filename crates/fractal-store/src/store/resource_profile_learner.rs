//! Host-local, retirement-qualified resource learning. Historical client receipts are not inputs.
//!
//! Samples are authoritative local observations, committed with terminal execution state. The
//! estimator reads a bounded trailing window; retained rows also provide permanent replay dedupe.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

pub mod disk;
pub mod cgroup;
pub mod evidence;

pub const MIN_SAMPLES: usize = 20;
pub const WINDOW_SAMPLES: usize = 256;

pub use fractal_runtime::resource_readiness::HostReadiness;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Coverage {
    Full,
    ClientOnly,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryEvents {
    pub max: u64,
    pub high: u64,
    pub oom: u64,
    pub oom_kill: u64,
    pub oom_group_kill: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    Natural,
    Cancelled,
    TimedOut,
    MaxHold,
    Signalled,
    #[default]
    Unknown,
}

/// Produced by the executing host only after generation-fenced whole-tree retirement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetirementSample {
    pub host: String,
    pub key: String,
    /// Changes when operation, parallelism, compiler profile or measurement coverage changes.
    pub semantic_revision: String,
    pub execution: String,
    pub generation: String,
    pub retirement_confirmed: bool,
    pub pre_retirement_populated: Option<bool>,
    pub end_reason: EndReason,
    pub coverage: Coverage,
    pub memory_peak_bytes: Option<u64>,
    pub memory_events: Option<MemoryEvents>,
    /// Allocated-byte high-water minus baseline, over exclusively owned output roots.
    /// None means incomplete attribution, not zero growth. Never supply io.stat write traffic.
    pub disk_growth_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rejection {
    UnconfirmedRetirement,
    Truncated,
    PopulatedBeforeRetirement,
    MissingRetirementEvidence,
    IncompleteCoverage,
    MissingMemoryEvents,
    MemoryLimitEvent,
    MissingMemoryPeak,
}

impl RetirementSample {
    pub fn rejection(&self) -> Option<Rejection> {
        if !self.retirement_confirmed {
            return Some(Rejection::UnconfirmedRetirement);
        }
        match self.pre_retirement_populated {
            Some(false) => {}
            Some(true) => return Some(Rejection::PopulatedBeforeRetirement),
            None => return Some(Rejection::MissingRetirementEvidence),
        }
        if self.end_reason != EndReason::Natural {
            return Some(Rejection::Truncated);
        }
        if self.coverage != Coverage::Full {
            return Some(Rejection::IncompleteCoverage);
        }
        let Some(events) = self.memory_events else {
            return Some(Rejection::MissingMemoryEvents);
        };
        if events.max != 0 || events.oom != 0 || events.oom_kill != 0 || events.oom_group_kill != 0 {
            return Some(Rejection::MemoryLimitEvent);
        }
        if self.memory_peak_bytes.is_none_or(|peak| peak == 0) {
            return Some(Rejection::MissingMemoryPeak);
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", content = "reason", rename_all = "kebab-case")]
pub enum RecordOutcome {
    Recorded,
    Duplicate,
    Rejected(Rejection),
}

pub fn create_schema(connection: &Connection) -> Result<()> {
    // All identifiers are validated below; counters are at most 20 decimal bytes. One immutable
    // row per qualified execution is retained, including outside the estimator's trailing window.
    // Transitional debt: retention does not bound sample-window growth or reclamation work.
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hac7950642ca2 owner=fractal_store::store::resource_profile_learner TODO=chunk-or-reference-variable-columns */
        "CREATE TABLE IF NOT EXISTS resource_profile_samples_v1 (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            host TEXT NOT NULL CHECK(length(host) BETWEEN 1 AND 128),
            profile_key TEXT NOT NULL CHECK(length(profile_key) BETWEEN 1 AND 256),
            semantic_revision TEXT NOT NULL CHECK(length(semantic_revision) BETWEEN 1 AND 128),
            execution TEXT NOT NULL CHECK(length(execution) BETWEEN 1 AND 256),
            generation TEXT NOT NULL CHECK(length(generation) BETWEEN 1 AND 256),
            memory_peak TEXT NOT NULL CHECK(length(memory_peak) BETWEEN 1 AND 20),
            disk_growth TEXT CHECK(length(disk_growth) BETWEEN 1 AND 20),
            UNIQUE(host, execution, generation)
        );
        CREATE INDEX IF NOT EXISTS resource_profile_samples_window_v1
            ON resource_profile_samples_v1(host, profile_key, semantic_revision, sequence DESC);",
    )?;
    Ok(())
}

fn validate_identity(host: &str, key: &str, revision: &str) -> Result<()> {
    ensure!(!host.is_empty() && host.len() <= 128, "invalid learner host identity");
    ensure!(!key.is_empty() && key.len() <= 256, "invalid learner profile key");
    ensure!(!revision.is_empty() && revision.len() <= 128, "invalid learner semantic revision");
    Ok(())
}

/// Call in the same transaction as the terminal receipt, never on a detached client exit path.
pub fn record_retirement_tx(
    transaction: &Transaction<'_>,
    sample: &RetirementSample,
) -> Result<RecordOutcome> {
    validate_identity(&sample.host, &sample.key, &sample.semantic_revision)?;
    ensure!(!sample.execution.is_empty() && sample.execution.len() <= 256,
        "invalid learner execution identity");
    ensure!(!sample.generation.is_empty() && sample.generation.len() <= 256,
        "invalid learner execution generation");
    if let Some(reason) = sample.rejection() {
        return Ok(RecordOutcome::Rejected(reason));
    }
    let peak = sample.memory_peak_bytes.context("qualified sample has no peak")?;
    let inserted = transaction.execute(
        // Transitional debt: sample rows are inline until reclamation gains a bounded driver.
        /* sqlite-large-value: allow-for-now lv-h753b486e3fbb owner=fractal_store::store::resource_profile_learner TODO=bound-or-reference-values-and-reclamation */
        "INSERT INTO resource_profile_samples_v1
            (host,profile_key,semantic_revision,execution,generation,memory_peak,disk_growth)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(host,execution,generation) DO NOTHING",
        params![sample.host, sample.key, sample.semantic_revision, sample.execution,
            sample.generation, peak.to_string(),
            sample.disk_growth_bytes.map(|bytes| bytes.to_string())],
    )?;
    if inserted != 0 {
        return Ok(RecordOutcome::Recorded);
    }
    let unchanged = transaction.query_row(
        "SELECT profile_key=?4 AND semantic_revision=?5 AND memory_peak=?6
            AND disk_growth IS ?7 FROM resource_profile_samples_v1
         WHERE host=?1 AND execution=?2 AND generation=?3",
        params![sample.host, sample.execution, sample.generation, sample.key,
            sample.semantic_revision, peak.to_string(),
            sample.disk_growth_bytes.map(|bytes| bytes.to_string())],
        |row| row.get::<_, bool>(0),
    )?;
    ensure!(unchanged, "conflicting retirement sample for execution generation");
    Ok(RecordOutcome::Duplicate)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LearnedEstimate {
    pub revision: Option<u64>,
    pub memory_samples: u64,
    pub disk_samples: u64,
    pub memory_high_bytes: u64,
    pub memory_max_bytes: u64,
    pub disk_bytes: u64,
    pub below_seed_ready: bool,
    pub memory_learned: bool,
    pub disk_learned: bool,
}

/// Exact upward-rounded 20% margin without a floating-point conversion or overflowing product.
pub fn memory_max(high: u64) -> Result<u64> {
    ensure!(high != 0, "memory high must be positive");
    high.checked_add(high / 5)
        .and_then(|max| max.checked_add(u64::from(high % 5 != 0)))
        .context("memory max overflows u64")
}

fn p95(values: &mut [u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let rank = values.len() - values.len() / 20 - 1;
    let (_, value, _) = values.select_nth_unstable(rank);
    Some(*value)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeedLimits {
    pub memory_high_bytes: u64,
    pub memory_max_bytes: u64,
    pub disk_bytes: u64,
}

/// Memory and disk learn independently; absent attributable disk never becomes a zero sample.
/// Readiness prevents shrinking either declaration while host attribution is incomplete.
pub fn estimate(
    connection: &Connection,
    host: &str,
    key: &str,
    semantic_revision: &str,
    seed: SeedLimits,
    readiness: HostReadiness,
) -> Result<LearnedEstimate> {
    validate_identity(host, key, semantic_revision)?;
    ensure!(seed.memory_high_bytes != 0 && seed.memory_max_bytes >= seed.memory_high_bytes,
        "seed requires positive memory high and max at least high");
    let mut statement = connection.prepare_cached(
        "SELECT sequence,memory_peak,disk_growth FROM resource_profile_samples_v1
         WHERE host=?1 AND profile_key=?2 AND semantic_revision=?3
         ORDER BY sequence DESC LIMIT ?4",
    )?;
    let mut rows = statement.query(params![host, key, semantic_revision, WINDOW_SAMPLES as u32])?;
    let mut memory = Vec::new();
    let mut disk = Vec::new();
    let mut revision = None;
    while let Some(row) = rows.next()? {
        let sequence: u64 = row.get(0)?;
        revision.get_or_insert(sequence);
        if memory.is_empty() { memory.reserve_exact(WINDOW_SAMPLES); }
        memory.push(row.get_ref(1)?.as_str()?.parse::<u64>()?);
        if let value @ rusqlite::types::ValueRef::Text(_) = row.get_ref(2)? {
            if disk.is_empty() { disk.reserve_exact(WINDOW_SAMPLES); }
            disk.push(value.as_str()?.parse::<u64>()?);
        } else {
            ensure!(matches!(row.get_ref(2)?, rusqlite::types::ValueRef::Null),
                "invalid persisted disk-growth counter");
        }
    }
    let memory_samples = u64::try_from(memory.len())?;
    let disk_samples = u64::try_from(disk.len())?;
    let memory_learned = memory.len() >= MIN_SAMPLES;
    let disk_learned = disk.len() >= MIN_SAMPLES;
    let ready = readiness.permits_below_seed();
    let memory_high_bytes = if memory_learned {
        let measured = p95(&mut memory).context("qualified memory window is empty")?;
        if ready { measured } else { measured.max(seed.memory_high_bytes) }
    } else {
        seed.memory_high_bytes
    };
    let memory_max_bytes = if memory_learned {
        let learned = memory_max(memory_high_bytes)?;
        if ready { learned } else { learned.max(seed.memory_max_bytes) }
    } else {
        seed.memory_max_bytes
    };
    let disk_bytes = if disk_learned {
        let measured = p95(&mut disk).context("qualified disk window is empty")?;
        if ready { measured } else { measured.max(seed.disk_bytes) }
    } else {
        seed.disk_bytes
    };
    Ok(LearnedEstimate {
        revision, memory_samples, disk_samples, memory_high_bytes,
        memory_max_bytes, disk_bytes,
        below_seed_ready: ready, memory_learned, disk_learned,
    })
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiskRemaining {
    pub remaining_bytes: u64,
    pub overshot: bool,
}

/// Host free space already reflects consumed allocation; reserve only unrealized growth.
/// An overshoot closes new admission; this value never authorizes terminating a holder.
pub fn remaining_disk(estimate: u64, allocated_high_water: u64) -> DiskRemaining {
    DiskRemaining {
        remaining_bytes: estimate.saturating_sub(allocated_high_water),
        overshot: allocated_high_water > estimate,
    }
}

#[cfg(test)]
#[path = "resource_profile_learner/tests.rs"]
mod tests;

impl super::Store {
    pub fn resource_profile_estimate(
        &self,
        host: &str,
        key: &str,
        semantic_revision: &str,
        seed: SeedLimits,
        readiness: HostReadiness,
    ) -> Result<LearnedEstimate> {
        let connection = self.readers.get();
        estimate(&connection, host, key, semantic_revision, seed, readiness)
    }
}
