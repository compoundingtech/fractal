//! Local authoritative roll registry. Receipts are immutable; reads never open the ledger.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, ensure};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::declared_objectives::{BudgetEvidence, classify_budget};
use crate::objective::{BudgetPolicy, BurnWindow};
use crate::store::Store;

const DAY: u64 = 86_400_000;
const OBJECTIVE: &str = "fleet/roll-completion";
const POLICY: BudgetPolicy = BudgetPolicy {
    allowance: 0.1, horizon_ms: 30 * DAY, min_coverage: 0.9, min_samples: 1,
    fast: BurnWindow { burn_rate: 14.4, long_ms: 3_600_000, short_ms: 300_000 },
    slow: BurnWindow { burn_rate: 6.0, long_ms: 21_600_000, short_ms: 1_800_000 },
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub dotfiles_revision: String,
    pub factory_revision: String,
    pub daemon_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_path: Option<String>,
}
impl Identity {
    fn validate(&self) -> Result<()> {
        ensure!([&self.dotfiles_revision, &self.factory_revision, &self.daemon_revision]
            .iter().all(|value| !value.trim().is_empty())
            && [&self.system_path, &self.home_path].iter().all(|value| value.as_ref().is_none_or(|path| !path.trim().is_empty())),
            "target revisions and any declared paths must be nonempty exact identities");
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub system_path: String,
    pub home_path: String,
    pub expected_system_path: String,
    pub expected_home_path: String,
    pub daemon_revision: String,
    pub factory_revision: String,
    pub daemon_ready: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Adoption { Verified, NotAdopted, Unknown }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Receipt {
    RollDeclared { version: u32, roll_id: String, declared_at_ms: u64, targets: BTreeMap<String, Identity> },
    HostObserved { version: u32, roll_id: String, declared_at_ms: u64, host: String,
        observed_at_ms: u64, target: Identity, adoption: Adoption, evidence: Option<Verification> },
}

#[derive(Default)]
struct Registry {
    rolls: BTreeMap<String, Roll>,
    accepted_at_ms: Option<u64>,
}
struct Roll {
    declared_at_ms: u64,
    targets: BTreeMap<String, Identity>,
    observations: BTreeMap<String, BTreeMap<u64, (Adoption, Option<Verification>)>>,
    first_verified: BTreeMap<String, u64>,
    completed_at_ms: Option<u64>,
    first_miss_at_ms: Option<u64>,
}
#[derive(Default)]
struct Source {
    writer: Mutex<Option<Connection>>,
    registry: RwLock<Registry>,
    installed: AtomicBool,
}

impl Registry {
    fn validate(&self, receipt: &Receipt, now: u64) -> Result<bool> {
        match receipt {
            Receipt::RollDeclared { version, roll_id, declared_at_ms, targets } => {
                validate_header(*version, roll_id, *declared_at_ms, now)?;
                ensure!(!targets.is_empty() && targets.keys().all(|host| !host.trim().is_empty()), "a roll needs its entire nonempty target cohort");
                for target in targets.values() { target.validate()?; }
                if let Some(roll) = self.rolls.get(roll_id) {
                    ensure!(roll.declared_at_ms == *declared_at_ms && roll.targets == *targets, "immutable roll declaration conflict");
                    return Ok(false);
                }
            }
            Receipt::HostObserved { version, roll_id, declared_at_ms, host, observed_at_ms, target, adoption, evidence } => {
                validate_header(*version, roll_id, *declared_at_ms, now)?;
                ensure!(*observed_at_ms >= *declared_at_ms && *observed_at_ms <= now, "observation must follow declaration and cannot be in the future");
                let roll = self.rolls.get(roll_id).context("declare the immutable roll before observations")?;
                ensure!(roll.declared_at_ms == *declared_at_ms && roll.targets.get(host) == Some(target), "observation does not match the immutable declared host target");
                if *adoption != Adoption::Unknown {
                    let proof = evidence.as_ref().context("known adoption requires complete independently observed activation and running daemon evidence")?;
                    ensure!([&proof.system_path, &proof.home_path, &proof.expected_system_path,
                        &proof.expected_home_path, &proof.daemon_revision, &proof.factory_revision]
                        .iter().all(|value| !value.trim().is_empty()), "partial adoption evidence is unknown");
                    let exact = proof.daemon_ready && proof.daemon_revision == target.daemon_revision
                        && proof.factory_revision == target.factory_revision
                        && proof.system_path == proof.expected_system_path && proof.home_path == proof.expected_home_path
                        && target.system_path.as_ref().is_none_or(|path| path == &proof.expected_system_path)
                        && target.home_path.as_ref().is_none_or(|path| path == &proof.expected_home_path);
                    ensure!(exact == (*adoption == Adoption::Verified),
                        "adoption label contradicts complete exact-target verification evidence");
                }
                if let Some(prior) = roll.observations.get(host).and_then(|observations| observations.get(observed_at_ms)) {
                    ensure!(prior.0 == *adoption && prior.1 == *evidence, "immutable host observation conflict");
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn record(&mut self, receipt: Receipt, now: u64) {
        match receipt {
            Receipt::RollDeclared { roll_id, declared_at_ms, targets, .. } => {
                self.rolls.insert(roll_id, Roll { declared_at_ms, targets, observations: BTreeMap::new(),
                    first_verified: BTreeMap::new(), completed_at_ms: None, first_miss_at_ms: None });
            }
            Receipt::HostObserved { roll_id, host, observed_at_ms, adoption, evidence, .. } => {
                let roll = self.rolls.get_mut(&roll_id).expect("validated declaration");
                if adoption == Adoption::Verified {
                    let verified = roll.first_verified.entry(host.clone()).or_insert(observed_at_ms);
                    *verified = (*verified).min(observed_at_ms);
                    if roll.first_verified.len() == roll.targets.len() {
                        roll.completed_at_ms = roll.first_verified.values().copied().max();
                    }
                } else if adoption == Adoption::NotAdopted && observed_at_ms > roll.declared_at_ms + DAY {
                    roll.first_miss_at_ms = Some(roll.first_miss_at_ms.unwrap_or(observed_at_ms).min(observed_at_ms));
                }
                roll.observations.entry(host).or_default().insert(observed_at_ms, (adoption, evidence));
            }
        }
        self.accepted_at_ms = Some(self.accepted_at_ms.unwrap_or(0).max(now));
    }

    fn accept(&mut self, receipt: Receipt, now: u64) -> Result<bool> {
        let changed = self.validate(&receipt, now)?;
        if changed { self.record(receipt, now); }
        Ok(changed)
    }
}
fn validate_header(version: u32, id: &str, declared: u64, now: u64) -> Result<()> {
    ensure!(version == 1, "unsupported roll receipt version");
    ensure!(!id.trim().is_empty() && declared <= now && declared.checked_add(DAY).is_some(), "invalid roll identity or declaration time");
    Ok(())
}

/// Open once before serving. This local authority is intentionally not replicated graph state.
pub fn install(store: &Store, state_dir: &Path) -> Result<()> {
    let source = store.fractal.extensions.get::<Source>();
    let mut writer = source.writer.lock().unwrap_or_else(PoisonError::into_inner);
    ensure!(writer.is_none(), "roll receipt registry already installed");
    let connection = Connection::open(state_dir.join("roll-receipts.sqlite3"))?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h965d880bf5ba owner=fractal_api_core::roll_completion TODO=chunk-or-reference-variable-columns */ "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS receipts (sequence INTEGER PRIMARY KEY, receipt_key TEXT NOT NULL UNIQUE, body TEXT NOT NULL, accepted_at_ms INTEGER NOT NULL);")?;
    let mut registry = Registry::default();
    {
        let mut statement = connection.prepare("SELECT body, accepted_at_ms FROM receipts ORDER BY sequence")?;
        let mut records = statement.query([])?;
        while let Some(record) = records.next()? {
            let body: String = record.get(0)?;
            registry.accept(serde_json::from_str(&body)?, record.get(1)?)?;
        }
    }
    *source.registry.write().unwrap_or_else(PoisonError::into_inner) = registry;
    *writer = Some(connection);
    source.installed.store(true, Ordering::Release);
    Ok(())
}

/// Validate and commit before publishing; a failed durable append never changes the projection.
pub fn ingest(store: &Store, receipt: Receipt, now: u64) -> Result<Value> {
    let source = store.fractal.extensions.get::<Source>();
    let mut writer = source.writer.lock().unwrap_or_else(PoisonError::into_inner);
    let changed = source.registry.read().unwrap_or_else(PoisonError::into_inner).validate(&receipt, now)
        .map_err(|error| crate::model::FractalError::new("invalid-roll-receipt", error.to_string()))?;
    let key = match &receipt {
        Receipt::RollDeclared { roll_id, .. } => json!([roll_id, "declaration"]),
        Receipt::HostObserved { roll_id, host, observed_at_ms, .. } => json!([roll_id, host, observed_at_ms]),
    }.to_string();
    let connection = writer.as_mut().context("roll receipt registry is not installed")?;
    if changed {
        let body = serde_json::to_string(&receipt)?;
        let transaction = connection.transaction()?;
        let existing: Option<String> = transaction.query_row("SELECT body FROM receipts WHERE receipt_key = ?1", [&key], |row| row.get(0)).optional()?;
        ensure!(existing.is_none(), "durable roll receipt conflicts with registry");
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h5d4b16b81b87 owner=fractal_api_core::roll_completion TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO receipts(receipt_key,body,accepted_at_ms) VALUES (?1,?2,?3)", params![key, body, now])?;
        transaction.commit()?;
        source.registry.write().unwrap_or_else(PoisonError::into_inner).record(receipt, now);
    }
    Ok(json!({"accepted": true, "changed": changed, "receipt_key": key}))
}

/// Final results never expire with a later unknown observation. Verification time, not an
/// inferred installation time, establishes completion. Missing evidence never proves a miss.
fn outcome(roll: &Roll, now: u64) -> Option<(u64, bool)> {
    let deadline = roll.declared_at_ms + DAY;
    if let Some(completed) = roll.completed_at_ms.filter(|at| *at <= now) {
        return Some((completed.min(deadline), completed > deadline));
    }
    roll.first_miss_at_ms.filter(|at| *at <= now).map(|_| (deadline, true))
}

/// Native source rows precede indicator.native evaluation; no filesystem/SQL in this path.
pub fn row(store: &Store, now: u64) -> Value {
    let source = store.fractal.extensions.get::<Source>();
    let registry = source.registry.read().unwrap_or_else(PoisonError::into_inner);
    render(&registry, source.installed.load(Ordering::Acquire), now)
}
fn render(registry: &Registry, installed: bool, now: u64) -> Value {
    let widths = [POLICY.horizon_ms, POLICY.fast.long_ms, POLICY.fast.short_ms, POLICY.slow.long_ms, POLICY.slow.short_ms];
    let mut counts = [(0_u64, 0_u64); 5];
    let mut unknown_by_window = [0_u64; 5];
    let mut unknown = 0;
    for roll in registry.rolls.values().filter(|roll| roll.declared_at_ms <= now) {
        if let Some((at, failed)) = outcome(roll, now) {
            for (width, (total, bad)) in widths.iter().zip(&mut counts) {
                if at >= now.saturating_sub(*width) && at <= now {
                    *total += 1;
                    *bad += u64::from(failed);
                }
            }
        } else {
            unknown += 1;
            let deadline = roll.declared_at_ms + DAY;
            if deadline <= now {
                for (width, unknown) in widths.iter().zip(&mut unknown_by_window) {
                    if deadline >= now.saturating_sub(*width) { *unknown += 1; }
                }
            }
        }
    }
    // Retain known outcome counts; overdue unknowns gate coverage, never become good/bad.
    let coverage = std::array::from_fn::<_, 5, _>(|index| {
        let total = counts[index].0;
        let possible = total + unknown_by_window[index];
        if possible == 0 { 0.0 } else { total as f64 / possible as f64 }
    });
    let evidence = std::array::from_fn::<_, 5, _>(|index| {
        let (total, bad) = counts[index];
        BudgetEvidence { bad: bad as f64, total: total as f64,
            eligible: installed && total >= POLICY.min_samples && coverage[index] >= POLICY.min_coverage }
    });
    let horizon = evidence[0];
    let (status, remaining) = classify_budget(&POLICY, horizon, (evidence[1], evidence[2]), (evidence[3], evidence[4]), "no-data");
    let windows: Vec<_> = widths.into_iter().enumerate().map(|(index, width)|
        json!({"window_ms": width, "bad": evidence[index].bad, "eligible": evidence[index].total,
            "unknown": unknown_by_window[index], "coverage": coverage[index]})).collect();
    json!({"id": OBJECTIVE, "kind": "objective", "about": "Every immutable target host verifies the exact declared roll within 24h",
        "target": "90% of fleet rolls complete within 24h; 10% bad-roll allowance", "current": format!("{} / {} known rolls late; {unknown} unknown open rolls", horizon.bad, horizon.total),
        "window": "30d", "remaining_percent": remaining, "status": status,
        "explain": "One denominator event per fleet roll, never per host. Holds never pause the declaration clock. Missing, stale, or unknown adoption is neither good nor bad. A post-deadline not-adopted observation proves a miss; late verification accounts at the deadline, timely completion at final verification. Fast 1h+5m at 14.4x; slow 6h+30m at 6x. Both windows must burn. Bind indicator native to fleet/roll-completion for owner episodes. Population is only declarations accepted by this local authority, not all external fleet activity.",
        "measurement": {"source": "local-native-roll-registry", "installed": installed, "as_of_unix_ms": now,
            "population": "accepted-declarations-only", "accepted_declarations": registry.rolls.len(), "last_receipt_at_ms": registry.accepted_at_ms,
            "population_frontier_ms": if installed { Some(now) } else { None }, "external_population_coverage": "unknown",
            "unknown_open_rolls": unknown, "windows": windows}})
}

#[cfg(test)]
#[path = "roll_completion_tests.rs"]
mod tests;
