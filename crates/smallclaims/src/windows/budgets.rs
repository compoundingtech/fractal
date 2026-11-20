//! Host-local compact SLO evidence. Request paths only increment counters; the minute
//! checkpoint commits closed receipts. Missing receipts (including restart partial minutes)
//! are unknown, never reconstructed from uptime or from a recent histogram.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

const MINUTE: u64 = 60_000;
const HORIZON: u64 = 30 * 24 * 60 * MINUTE;
const WINDOWS: [(&str, u64); 6] = [
    ("30d", HORIZON), ("6h", 6 * 60 * MINUTE), ("2h", 2 * 60 * MINUTE),
    ("1h", 60 * MINUTE), ("30m", 30 * MINUTE), ("5m", 5 * MINUTE),
];

#[derive(Default)]
struct Counts { eligible: u64, bad: u64 }

struct Ledger {
    db: Connection,
    streams: Arc<[String]>,
    pending: BTreeMap<(usize, u64), Counts>,
    started_ms: u64,
    last_ms: u64,
    last_instant: Instant,
    committed_through_ms: u64,
}

static LEDGER: Mutex<Option<Ledger>> = Mutex::new(None);
static RECORDER: OnceLock<Recorder> = OnceLock::new();

// Writers build the entire immutable view while owning LEDGER. Readers only hold
// this independent lock long enough to clone an Arc, never across SQL or rendering.
static PROJECTION: RwLock<Option<Arc<Projection>>> = RwLock::new(None);

struct Projection {
    as_of_unix_ms: u64,
    cut_ms: u64,
    committed_through_ms: u64,
    valid: bool,
    events: BTreeMap<String, Value>,
    gauges: BTreeMap<String, Vec<(u64, Option<f64>)>>,
}

fn projection_snapshot(store: &RwLock<Option<Arc<Projection>>>) -> Option<Arc<Projection>> {
    store.read().unwrap_or_else(PoisonError::into_inner).clone()
}

fn swap_projection(store: &RwLock<Option<Arc<Projection>>>, next: Option<Arc<Projection>>) {
    let previous = {
        let mut guard = store.write().unwrap_or_else(PoisonError::into_inner);
        std::mem::replace(&mut *guard, next)
    };
    // Retained gauge/history storage is reclaimed outside the reader's short lock.
    drop(previous);
}

/// All ledger mutations, including their reads and summary rebuild, run inside this
/// boundary. Every error invalidates the published view before returning to its caller.
fn publish_mutation<T>(
    store: &RwLock<Option<Arc<Projection>>>,
    mutation: impl FnOnce() -> Result<(T, Projection)>,
) -> Result<T> {
    match mutation() {
        Ok((output, projection)) => {
            let projection = Arc::new(projection);
            swap_projection(store, Some(projection));
            Ok(output)
        }
        Err(error) => {
            // A durable mutation may already have succeeded. An older same-cut
            // green view is no longer authoritative, but failure must not advance as-of.
            let previous = projection_snapshot(store);
            let unavailable = previous.map(|previous| Arc::new(Projection {
                as_of_unix_ms: previous.as_of_unix_ms, cut_ms: previous.cut_ms,
                committed_through_ms: previous.committed_through_ms, valid: false,
                events: BTreeMap::new(), gauges: BTreeMap::new(),
            }));
            swap_projection(store, unavailable);
            Err(error)
        }
    }
}

/// The completion path shares only this in-memory buffer, never the ledger's I/O mutex.
struct Recorder {
    streams: Arc<[String]>,
    pending: Mutex<BTreeMap<(u64, usize), Counts>>,
}

impl Recorder {
    fn record(&self, stream: &str, bad: bool) {
        let Some(index) = self.streams.iter().position(|s| s == stream) else { return; };
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let now = unix_ms();
        let key = (now / MINUTE, index);
        if !pending.contains_key(&key) {
            // Each expired bucket is removed once, rather than scanning retained history.
            while pending.first_key_value().is_some_and(|((slot, _), _)|
                slot.saturating_mul(MINUTE) < now.saturating_sub(HORIZON)) {
                pending.pop_first();
            }
        }
        let counts = pending.entry(key).or_default();
        counts.eligible = counts.eligible.saturating_add(1);
        counts.bad = counts.bad.saturating_add(u64::from(bad));
    }
}

pub fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
        .as_millis().min(u128::from(u64::MAX)) as u64
}

/// Open an exclusive host-local namespace. Stream identities include the measurement
/// boundary, so a changed threshold cannot reinterpret old bad counters.
pub fn install(path: &Path, node: &str, streams: Vec<String>) -> Result<()> {
    ensure!(streams.iter().all(|stream| !stream.is_empty() && stream.len() <= 256),
        "native SLO stream identity must contain 1..=256 bytes");
    let mut guard = LEDGER.lock().unwrap_or_else(PoisonError::into_inner);
    ensure!(guard.is_none(), "native SLO evidence is already installed");
    ensure!(RECORDER.get().is_none(), "native SLO recorder is already installed");
    let streams: Arc<[String]> = streams.into();
    let ledger = publish_mutation(&PROJECTION, || {
        let db = open(path, node)?;
        let now = unix_ms();
        db.execute(/* sqlite-large-value: allow-for-now lv-h1600ef705393 owner=smallclaims::windows::budgets TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM native_slo_minutes_v1 WHERE slot >= ?1", [now / MINUTE])?;
        db.execute(/* sqlite-large-value: allow-for-now lv-h0a7bcaadb3af owner=smallclaims::windows::budgets TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM native_slo_gauges_v1 WHERE at_ms > ?1", [now])?;
        let ledger = Ledger { db, streams: Arc::clone(&streams), pending: BTreeMap::new(), started_ms: now,
            last_ms: now, last_instant: Instant::now(), committed_through_ms: now / MINUTE * MINUTE };
        let projection = ledger.projection_at(now)?;
        Ok((ledger, projection))
    })?;
    // LEDGER serializes initialization and the precondition above excludes prior setup.
    RECORDER.set(Recorder { streams, pending: Mutex::new(BTreeMap::new()) })
        .unwrap_or_else(|_| unreachable!("native SLO recorder initialization is serialized"));
    *guard = Some(ledger);
    Ok(())
}

fn open(path: &Path, node: &str) -> Result<Connection> {
    ensure!(!node.is_empty() && node.len() <= 256,
        "native SLO node identity must contain 1..=256 bytes");
    let mut db = Connection::open(path)?;
    crate::sqlite::census_files::register(&db);
    db.execute_batch(/* sqlite-large-value: allow-for-now lv-h7d555c9c34a2 owner=smallclaims::windows::budgets TODO=chunk-or-reference-variable-columns */ "PRAGMA synchronous = FULL;
        CREATE TABLE IF NOT EXISTS native_slo_owner_v1 (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            node TEXT NOT NULL CHECK(length(CAST(node AS BLOB)) BETWEEN 1 AND 256)
        ) STRICT;
        CREATE TABLE IF NOT EXISTS native_slo_minutes_v1 (
            stream TEXT NOT NULL CHECK(length(CAST(stream AS BLOB)) BETWEEN 1 AND 256),
            slot INTEGER NOT NULL CHECK(slot >= 0),
            eligible INTEGER NOT NULL CHECK(eligible >= 0),
            bad INTEGER NOT NULL CHECK(bad >= 0 AND bad <= eligible),
            PRIMARY KEY(stream, slot)
        ) STRICT;
        CREATE TABLE IF NOT EXISTS native_slo_gauges_v1 (
            stream TEXT NOT NULL CHECK(length(CAST(stream AS BLOB)) BETWEEN 1 AND 256),
            at_ms INTEGER NOT NULL CHECK(at_ms >= 0),
            value REAL, PRIMARY KEY(stream, at_ms)
        ) STRICT;")?;
    let tx = db.transaction()?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h64986a2ae91d owner=smallclaims::windows::budgets TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO native_slo_owner_v1 VALUES(1, ?1)", [node])?;
    let owner: String = tx.query_row("SELECT node FROM native_slo_owner_v1 WHERE singleton=1", [], |r| r.get(0))?;
    ensure!(owner == node, "native SLO evidence belongs to another node");
    tx.commit()?;
    Ok(db)
}

/// No disk I/O on the measurement path. Only commissioned stream names are accepted.
pub fn record(stream: &str, bad: bool) {
    if let Some(recorder) = RECORDER.get() { recorder.record(stream, bad); }
}

/// Persist only a fully observed preceding minute. A late checkpoint, clock discontinuity,
/// or restart leaves a gap. Repeated checkpoints within a minute never duplicate counts.
pub fn checkpoint() -> Result<()> {
    let Some(recorder) = RECORDER.get() else { return Ok(()); };
    checkpoint_with(recorder, &LEDGER, &PROJECTION)
}

fn checkpoint_with(recorder: &Recorder, ledger: &Mutex<Option<Ledger>>, projection: &RwLock<Option<Arc<Projection>>>) -> Result<()> {
    let mut guard = ledger.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(ledger) = guard.as_mut() else { return Ok(()); };
    let (pending, now, instant) = {
        let mut buffer = recorder.pending.lock().unwrap_or_else(PoisonError::into_inner);
        (std::mem::take(&mut *buffer), unix_ms(), Instant::now())
    };
    // Merge outside the recorder lock. A failed write retains its snapshot for the next
    // checkpoint, while completions append independently to the new live buffer.
    for ((slot, index), delta) in pending {
        let counts = ledger.pending.entry((index, slot)).or_default();
        counts.eligible = counts.eligible.saturating_add(delta.eligible);
        counts.bad = counts.bad.saturating_add(delta.bad);
    }
    publish_mutation(projection, || {
        ledger.checkpoint_at(now, instant)?;
        Ok(((), ledger.projection_at(now)?))
    })
}

/// Configure the actual SQLite wait boundary for isolated cross-crate I/O regressions.
#[cfg(feature = "test-support")]
pub fn set_busy_handler_for_tests(handler: Option<fn(i32) -> bool>) -> Result<()> {
    let guard = LEDGER.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(ledger) = guard.as_ref() else { anyhow::bail!("native SLO ledger is not installed"); };
    ledger.db.busy_handler(handler)?;
    Ok(())
}

impl Ledger {
    fn projection_at(&self, now: u64) -> Result<Projection> {
        let mut streams = self.db.prepare("SELECT DISTINCT stream FROM native_slo_minutes_v1")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        streams.extend(self.streams.iter().cloned());
        streams.sort_unstable();
        streams.dedup();
        let mut events = BTreeMap::new();
        for stream in streams {
            let evidence = summarize_event_at(self, &stream, now)?;
            events.insert(stream, evidence);
        }
        let mut gauges: BTreeMap<String, Vec<(u64, Option<f64>)>> = BTreeMap::new();
        let mut query = self.db.prepare("SELECT stream,at_ms,value FROM native_slo_gauges_v1 ORDER BY stream,at_ms")?;
        let samples = query.query_map([], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, u64>(1)?, row.get::<_, Option<f64>>(2)?,
        )))?;
        for sample in samples {
            let (stream, at, value) = sample?;
            gauges.entry(stream).or_default().push((at, value));
        }
        Ok(Projection {
            as_of_unix_ms: now, cut_ms: now / MINUTE * MINUTE,
            committed_through_ms: self.committed_through_ms, valid: true, events, gauges,
        })
    }
    fn checkpoint_at(&mut self, now: u64, instant: Instant) -> Result<()> {
        let end = now / MINUTE * MINUTE;
        let elapsed = instant.saturating_duration_since(self.last_instant).as_millis() as u64;
        let continuous = now >= self.last_ms && now - self.last_ms <= 90_000
            && (now - self.last_ms).abs_diff(elapsed) <= 2_000;
        let start = end.saturating_sub(MINUTE);
        let complete = continuous && self.started_ms <= start;
        let tx = self.db.transaction()?;
        if end > self.committed_through_ms && complete {
            for (index, stream) in self.streams.iter().enumerate() {
                let counts = self.pending.get(&(index, start / MINUTE));
                let eligible = counts.map_or(0, |c| c.eligible);
                let bad = counts.map_or(0, |c| c.bad);
                let prior: Option<(u64, u64)> = tx.query_row(
                    "SELECT eligible,bad FROM native_slo_minutes_v1 WHERE stream=?1 AND slot=?2",
                    params![stream, start / MINUTE], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
                ensure!(prior.is_none_or(|p| p == (eligible, bad)), "conflicting native SLO minute receipt");
                tx.execute(/* sqlite-large-value: allow-for-now lv-h4d65f8fccc90 owner=smallclaims::windows::budgets TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO native_slo_minutes_v1 VALUES(?1,?2,?3,?4)",
                    params![stream, start / MINUTE, eligible, bad])?;
            }
        }
        if now < self.last_ms || !continuous {
            // Old receipts from a clock's future must not become evidence again later.
            tx.execute(/* sqlite-large-value: allow-for-now lv-hb93ffd756510 owner=smallclaims::windows::budgets TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM native_slo_minutes_v1 WHERE slot >= ?1", [start / MINUTE])?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h473cc127dd19 owner=smallclaims::windows::budgets TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM native_slo_gauges_v1 WHERE at_ms > ?1", [now])?;
            self.pending.clear();
            self.started_ms = now;
        }
        tx.execute(/* sqlite-large-value: allow-for-now lv-h5b9949116852 owner=smallclaims::windows::budgets TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM native_slo_minutes_v1 WHERE slot < ?1", [end.saturating_sub(HORIZON) / MINUTE])?;
        // Two hours of predecessor allowance is sufficient for the hourly native gauge.
        tx.execute(/* sqlite-large-value: allow-for-now lv-h4403777bcdfb owner=smallclaims::windows::budgets TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM native_slo_gauges_v1 WHERE at_ms < ?1",
            [now.saturating_sub(HORIZON + 2 * 60 * MINUTE)])?;
        tx.commit()?;
        self.pending.retain(|(_, slot), _| *slot >= end / MINUTE);
        self.committed_through_ms = end;
        self.last_ms = now;
        self.last_instant = instant;
        Ok(())
    }
}

/// Evidence, not a verdict. Windows advance with UTC time even if checkpointing stops.
/// Incomplete source coverage cannot establish an event denominator for the missing time.
pub fn report(stream: &str) -> Result<Value> {
    let projection = projection_snapshot(&PROJECTION);
    Ok(report_at(projection.as_deref(), stream, unix_ms()))
}

fn summarize_event_at(ledger: &Ledger, stream: &str, now_ms: u64) -> Result<Value> {
    let end = now_ms / MINUTE * MINUTE;
    let newest_slot: Option<u64> = ledger.db.query_row(
        "SELECT max(slot) FROM native_slo_minutes_v1 WHERE stream=?1 AND slot<?2",
        params![stream, end / MINUTE], |row| row.get(0))?;
    let mut windows = serde_json::Map::new();
    for (name, width) in WINDOWS {
        let (minutes, eligible, bad): (u64, u64, u64) = ledger.db.query_row(
            "SELECT count(*),coalesce(sum(eligible),0),coalesce(sum(bad),0)
             FROM native_slo_minutes_v1 WHERE stream=?1 AND slot>=?2 AND slot<?3",
            params![stream, end.saturating_sub(width) / MINUTE, end / MINUTE],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        windows.insert(name.to_owned(), json!({
            "eligible": eligible, "bad": bad, "good": eligible.saturating_sub(bad),
            "covered_ms": minutes * MINUTE, "unknown_ms": width.saturating_sub(minutes * MINUTE),
            "receipt_count": minutes,
            "window_ms": width, "from_unix_ms": end.saturating_sub(width), "through_unix_ms": end,
            "coverage": minutes as f64 * MINUTE as f64 / width as f64,
        }));
    }
    Ok(json!({"available": true, "source": "native-minute-ledger-v1", "stream": stream,
        "through_unix_ms": end, "committed_through_unix_ms": ledger.committed_through_ms,
        "newest_receipt_through_unix_ms": newest_slot.map(|slot| (slot + 1) * MINUTE), "windows": windows}))
}

fn unknown_report(projection: Option<&Projection>, stream: &str, end: u64, source: &str, reason: &str) -> Value {
    let windows = WINDOWS.into_iter().map(|(name, width)| (name.to_owned(), json!({
        "eligible": 0, "bad": 0, "good": 0, "covered_ms": 0, "unknown_ms": width,
        "receipt_count": 0, "samples": 0, "window_ms": width, "coverage": 0.0,
        "from_unix_ms": end.saturating_sub(width), "through_unix_ms": end,
    }))).collect::<serde_json::Map<_, _>>();
    json!({"available": false, "source": source, "stream": stream, "stale": true,
        "unavailable_reason": reason, "coverage": "unknown",
        "as_of_unix_ms": projection.map(|p| p.as_of_unix_ms),
        "accounting_cut_unix_ms": projection.map(|p| p.cut_ms),
        "committed_through_unix_ms": projection.map(|p| p.committed_through_ms),
        "through_unix_ms": end, "windows": windows})
}

fn unavailable_reason(projection: Option<&Projection>, now: u64) -> Option<&'static str> {
    let Some(projection) = projection else { return Some("projection_unavailable"); };
    if !projection.valid { return Some("projection_unavailable"); }
    if now < projection.as_of_unix_ms { return Some("clock_regressed"); }
    if now / MINUTE * MINUTE != projection.cut_ms { return Some("projection_stale"); }
    None
}

fn report_at(projection: Option<&Projection>, stream: &str, now: u64) -> Value {
    if let Some(reason) = unavailable_reason(projection, now) {
        return unknown_report(projection, stream, now / MINUTE * MINUTE, "native-minute-ledger-v1", reason);
    }
    let projection = projection.expect("fresh projection exists");
    let Some(evidence) = projection.events.get(stream) else {
        return unknown_report(Some(projection), stream, now / MINUTE * MINUTE, "native-minute-ledger-v1", "stream_unavailable");
    };
    let mut evidence = evidence.clone();
    evidence["as_of_unix_ms"] = json!(projection.as_of_unix_ms);
    evidence["accounting_cut_unix_ms"] = json!(projection.cut_ms);
    evidence["stale"] = json!(false);
    evidence
}

/// Gauge samples are infrequent source observations, not request-path operations.
/// Persist the source timestamp, never the time somebody read the newest sample.
pub fn record_gauge(stream: &str, at_ms: u64, value: Option<f64>) -> Result<()> {
    ensure!(!stream.is_empty() && stream.len() <= 256,
        "native SLO stream identity must contain 1..=256 bytes");
    ensure!(value.is_none_or(f64::is_finite), "native SLO gauge must be finite");
    let guard = LEDGER.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(ledger) = guard.as_ref() else { return Ok(()); };
    record_gauge_with(ledger, &PROJECTION, stream, at_ms, value)
}

fn record_gauge_with(
    ledger: &Ledger, projection: &RwLock<Option<Arc<Projection>>>,
    stream: &str, at_ms: u64, value: Option<f64>,
) -> Result<()> {
    publish_mutation(projection, || {
        let prior: Option<Option<f64>> = ledger.db.query_row("SELECT value FROM native_slo_gauges_v1 WHERE stream=?1 AND at_ms=?2",
            params![stream, at_ms], |r| r.get(0)).optional()?;
        ensure!(prior.is_none_or(|p| p == value), "conflicting native SLO gauge sample");
        ledger.db.execute(/* sqlite-large-value: allow-for-now lv-ha0bcfcb764ac owner=smallclaims::windows::budgets TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO native_slo_gauges_v1 VALUES(?1,?2,?3)", params![stream, at_ms, value])?;
        Ok(((), ledger.projection_at(unix_ms())?))
    })
}

/// Integrate finite forward holds, clipped by the next observation and maximum age.
/// Missing time remains unknown, not healthy or an extrapolated event rate.
pub fn gauge_report(stream: &str, now: u64, max_age_ms: u64, bad_above: f64) -> Result<Value> {
    let projection = projection_snapshot(&PROJECTION);
    Ok(gauge_report_at(projection.as_deref(), stream, now, max_age_ms, bad_above))
}

fn gauge_report_at(projection: Option<&Projection>, stream: &str, now: u64, max_age_ms: u64, bad_above: f64) -> Value {
    if let Some(reason) = unavailable_reason(projection, now) {
        return unknown_report(projection, stream, now, "native-gauge-ledger-v1", reason);
    }
    let projection = projection.expect("fresh projection exists");
    let samples = projection.gauges.get(stream).map_or(&[][..], Vec::as_slice);
    let mut windows = serde_json::Map::new();
    for (name, width) in WINDOWS {
        let (covered, bad, contributing_samples) = integrate(samples, now.saturating_sub(width), now, max_age_ms, bad_above);
        windows.insert(name.to_owned(), json!({"eligible": covered, "bad": bad, "good": covered - bad,
            "covered_ms": covered, "unknown_ms": width.saturating_sub(covered), "window_ms": width,
            "samples": contributing_samples,
            "from_unix_ms": now.saturating_sub(width), "through_unix_ms": now,
            "coverage": covered as f64 / width as f64}));
    }
    json!({"available": true, "source": "native-gauge-ledger-v1", "stream": stream,
        "as_of_unix_ms": projection.as_of_unix_ms, "accounting_cut_unix_ms": projection.cut_ms, "stale": false,
        "through_unix_ms": samples.iter().rev().find(|(at, _)| *at <= now).map(|(at, _)| at),
        "max_age_ms": max_age_ms, "windows": windows})
}

fn integrate(samples: &[(u64, Option<f64>)], from: u64, through: u64, max_age: u64, threshold: f64) -> (u64, u64, u64) {
    let mut covered = 0;
    let mut bad = 0;
    let mut contributing_samples = 0;
    for (index, &(at, value)) in samples.iter().enumerate() {
        let Some(value) = value else { continue; };
        let end = samples.get(index + 1).map_or(through, |(next, _)| *next)
            .min(at.saturating_add(max_age)).min(through);
        let held = end.saturating_sub(at.max(from));
        contributing_samples += u64::from(held > 0);
        covered += held;
        if value > threshold { bad += held; }
    }
    (covered, bad, contributing_samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection_ledger() -> Result<Ledger> {
        let now = 6 * MINUTE + 100;
        let ledger = Ledger {
            db: open(Path::new(":memory:"), "node")?, streams: vec!["x".into()].into(),
            pending: BTreeMap::new(), started_ms: 0, last_ms: now,
            last_instant: Instant::now(), committed_through_ms: 6 * MINUTE,
        };
        for slot in 1..=5 {
            ledger.db.execute("INSERT INTO native_slo_minutes_v1 VALUES('x',?1,10,0)", [slot])?;
        }
        ledger.db.execute("INSERT INTO native_slo_gauges_v1 VALUES('g',?1,2)", [now - 1_000])?;
        ledger.db.execute("INSERT INTO native_slo_gauges_v1 VALUES('g',?1,NULL)", [now - 500])?;
        ledger.db.execute("INSERT INTO native_slo_gauges_v1 VALUES('g',?1,9)", [now + 5_000])?;
        Ok(ledger)
    }

    #[test]
    fn fresh_projection_survives_ledger_destruction_without_reader_io() -> Result<()> {
        let ledger = projection_ledger()?;
        let now = ledger.last_ms;
        let store = RwLock::new(None);
        publish_mutation(&store, || Ok(((), ledger.projection_at(now)?)))?;
        drop(ledger);
        let projection = projection_snapshot(&store).unwrap();
        let report = report_at(Some(&projection), "x", now + 1);
        assert_eq!(report["available"], true);
        assert_eq!(report["stale"], false);
        assert_eq!(report["as_of_unix_ms"], now);
        assert_eq!(report["windows"]["5m"]["covered_ms"], 5 * MINUTE);
        assert_eq!(report["windows"]["5m"]["eligible"], 50);
        assert_eq!(report["windows"]["30d"]["unknown_ms"], HORIZON - 5 * MINUTE);
        Ok(())
    }

    #[test]
    fn projection_next_cut_and_clock_regression_are_explicitly_unknown() -> Result<()> {
        let ledger = projection_ledger()?;
        let now = ledger.last_ms;
        let projection = ledger.projection_at(now)?;
        for (at, reason) in [(7 * MINUTE, "projection_stale"), (now - 1, "clock_regressed")] {
            for report in [
                report_at(Some(&projection), "x", at),
                gauge_report_at(Some(&projection), "g", at, MINUTE, 1.0),
            ] {
                assert_eq!(report["available"], false);
                assert_eq!(report["coverage"], "unknown");
                assert_eq!(report["unavailable_reason"], reason);
                assert_eq!(report["as_of_unix_ms"], now);
                for (name, width) in WINDOWS {
                    assert_eq!(report["windows"][name]["covered_ms"], 0);
                    assert_eq!(report["windows"][name]["unknown_ms"], width);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn gauge_projection_integrates_caller_age_and_threshold_from_memory() -> Result<()> {
        let ledger = projection_ledger()?;
        let now = ledger.last_ms;
        let projection = ledger.projection_at(now)?;
        drop(ledger);
        let report = gauge_report_at(Some(&projection), "g", now, MINUTE, 1.0);
        assert_eq!(report["available"], true);
        assert_eq!(report["through_unix_ms"], now - 500);
        assert_eq!(report["windows"]["5m"]["covered_ms"], 500);
        assert_eq!(report["windows"]["5m"]["bad"], 500);
        assert_eq!(report["windows"]["5m"]["samples"], 1);
        let expired = gauge_report_at(Some(&projection), "g", now, 100, 3.0);
        assert_eq!(expired["windows"]["5m"]["covered_ms"], 100);
        assert_eq!(expired["windows"]["5m"]["bad"], 0);
        Ok(())
    }

    #[test]
    fn same_cut_checkpoint_publishes_asof_without_inventing_receipts() -> Result<()> {
        let mut ledger = projection_ledger()?;
        let now = ledger.last_ms;
        let store = RwLock::new(None);
        publish_mutation(&store, || Ok(((), ledger.projection_at(now)?)))?;
        let before = projection_snapshot(&store).unwrap();
        ledger.checkpoint_at(now + 100, ledger.last_instant + std::time::Duration::from_millis(100))?;
        publish_mutation(&store, || Ok(((), ledger.projection_at(now + 100)?)))?;
        let after = projection_snapshot(&store).unwrap();
        assert_eq!(after.as_of_unix_ms, now + 100);
        assert_eq!(after.cut_ms, before.cut_ms);
        assert_eq!(after.events["x"]["windows"], before.events["x"]["windows"]);
        assert_eq!(before.as_of_unix_ms, now);
        Ok(())
    }

    #[test]
    fn failed_summary_after_mutation_invalidates_same_cut_without_advancing_asof() -> Result<()> {
        let ledger = projection_ledger()?;
        let now = ledger.last_ms;
        let store = RwLock::new(None);
        publish_mutation(&store, || Ok(((), ledger.projection_at(now)?)))?;
        let before = projection_snapshot(&store).unwrap();
        ledger.db.execute("INSERT INTO native_slo_gauges_v1 VALUES('g',?1,100)", [now])?;
        ledger.db.execute_batch("DROP TABLE native_slo_minutes_v1")?;
        assert!(publish_mutation(&store, || Ok(((), ledger.projection_at(now + 100)?))).is_err());
        let after = projection_snapshot(&store).unwrap();
        assert_eq!(after.as_of_unix_ms, now);
        assert_eq!(after.cut_ms, before.cut_ms);
        assert_eq!(report_at(Some(&after), "x", now + 100)["available"], false);
        let gauge = gauge_report_at(Some(&after), "g", now + 100, MINUTE, 1.0);
        assert_eq!(gauge["available"], false);
        assert_eq!(gauge["unavailable_reason"], "projection_unavailable");
        assert_eq!(gauge["windows"]["5m"]["unknown_ms"], 5 * MINUTE);
        assert_eq!(report_at(Some(&before), "x", now)["available"], true);
        Ok(())
    }

    #[test]
    fn gauge_select_and_insert_failures_invalidate_healthy_same_cut_evidence() -> Result<()> {
        for failure in ["PRAGMA query_only=ON", "DROP TABLE native_slo_gauges_v1"] {
            let ledger = projection_ledger()?;
            let now = ledger.last_ms;
            ledger.db.execute("DELETE FROM native_slo_gauges_v1", [])?;
            ledger.db.execute("INSERT INTO native_slo_gauges_v1 VALUES('g',?1,2)", [now - 5 * MINUTE])?;
            let store = RwLock::new(None);
            publish_mutation(&store, || Ok(((), ledger.projection_at(now)?)))?;
            let before = projection_snapshot(&store).unwrap();
            let healthy = gauge_report_at(Some(&before), "g", now, 5 * MINUTE, 3.0);
            assert_eq!(healthy["available"], true);
            assert_eq!(healthy["windows"]["5m"]["bad"], 0);
            assert_eq!(healthy["windows"]["5m"]["covered_ms"], 5 * MINUTE);

            // Real SQLite failures: SELECT succeeds but INSERT is read-only in the
            // first case; SELECT itself fails on the missing table in the second.
            ledger.db.execute_batch(failure)?;
            let error = record_gauge_with(&ledger, &store, "g", now, Some(100.0)).unwrap_err();
            assert!(error.downcast_ref::<rusqlite::Error>().is_some(), "{error:#}");
            let after = projection_snapshot(&store).unwrap();
            assert_eq!(after.as_of_unix_ms, before.as_of_unix_ms);
            assert_eq!(after.cut_ms, before.cut_ms);
            let unknown = gauge_report_at(Some(&after), "g", now, 5 * MINUTE, 3.0);
            assert_eq!(unknown["available"], false);
            assert_eq!(unknown["unavailable_reason"], "projection_unavailable");
            assert_eq!(unknown["windows"]["5m"]["covered_ms"], 0);
            assert_eq!(unknown["windows"]["5m"]["unknown_ms"], 5 * MINUTE);
            assert_eq!(report_at(Some(&after), "x", now)["available"], false);
        }
        Ok(())
    }

    #[test]
    fn gauge_holds_expire_and_clip_without_filling_gaps() {
        assert_eq!(integrate(&[(0, Some(2.0)), (200, Some(0.0))], 50, 400, 100, 1.0), (150, 50, 2));
        assert_eq!(integrate(&[(0, Some(1.0))], 0, 100, 100, 1.0), (100, 0, 1));
        assert_eq!(integrate(&[(0, Some(0.0)), (50, None)], 0, 100, 100, 1.0), (50, 0, 1));
    }

    #[test]
    fn predecessor_readings_count_only_when_their_hold_contributes_to_the_window() {
        let readings = [(0, Some(2.0)), (200, Some(0.0)), (400, Some(2.0))];
        assert_eq!(integrate(&readings, 250, 400, 200, 1.0), (150, 0, 1));
        assert_eq!(integrate(&readings, 250, 400, 40, 1.0), (0, 0, 0));
        assert_eq!(integrate(&[(200, None)], 250, 400, 200, 1.0), (0, 0, 0));
    }

    #[test]
    fn durable_receipts_survive_reopen_but_restart_minutes_are_unknown() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("native.sqlite");
        let instant = Instant::now();
        let mut ledger = Ledger { db: open(&path, "node-a")?, streams: vec!["read/300".into()].into(),
            pending: BTreeMap::new(), started_ms: 0, last_ms: 0, last_instant: instant, committed_through_ms: 0 };
        ledger.pending.insert((0, 0), Counts { eligible: 100, bad: 5 });
        ledger.checkpoint_at(MINUTE, instant + std::time::Duration::from_secs(60))?;
        drop(ledger);
        let db = open(&path, "node-a")?;
        let count: u64 = db.query_row("SELECT eligible FROM native_slo_minutes_v1", [], |r| r.get(0))?;
        assert_eq!(count, 100);
        assert!(open(&path, "node-b").is_err());
        let mut restarted = Ledger { db, streams: vec!["read/300".into()].into(), pending: BTreeMap::new(),
            started_ms: MINUTE + 10_000, last_ms: MINUTE + 10_000, last_instant: instant,
            committed_through_ms: MINUTE };
        restarted.checkpoint_at(2 * MINUTE, instant + std::time::Duration::from_secs(50))?;
        let receipts: u64 = restarted.db.query_row("SELECT count(*) FROM native_slo_minutes_v1", [], |r| r.get(0))?;
        assert_eq!(receipts, 1);
        Ok(())
    }

    #[test]
    fn late_checkpoint_does_not_invent_quiet_coverage() -> Result<()> {
        let instant = Instant::now();
        let mut ledger = Ledger { db: open(Path::new(":memory:"), "node")?, streams: vec!["x".into()].into(),
            pending: BTreeMap::new(), started_ms: 0, last_ms: 0, last_instant: instant, committed_through_ms: 0 };
        ledger.checkpoint_at(3 * MINUTE, instant + std::time::Duration::from_secs(180))?;
        let count: u64 = ledger.db.query_row("SELECT count(*) FROM native_slo_minutes_v1", [], |r| r.get(0))?;
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn thirty_day_retention_keeps_the_real_oldest_in_window_receipt() -> Result<()> {
        let instant = Instant::now();
        let now = HORIZON + MINUTE;
        let mut ledger = Ledger { db: open(Path::new(":memory:"), "node")?, streams: vec!["x".into()].into(),
            pending: BTreeMap::new(), started_ms: 0, last_ms: now - MINUTE, last_instant: instant,
            committed_through_ms: now - MINUTE };
        for (slot, count) in [(0, 999), (1, 20), (100, 30)] {
            ledger.db.execute("INSERT INTO native_slo_minutes_v1 VALUES('x',?1,?2,0)", params![slot, count])?;
        }
        ledger.checkpoint_at(now, instant + std::time::Duration::from_secs(60))?;
        let eligible: u64 = ledger.db.query_row("SELECT sum(eligible) FROM native_slo_minutes_v1", [], |r| r.get(0))?;
        assert_eq!(eligible, 50);
        // A second checkpoint at the same cut cannot create a duplicate receipt.
        ledger.checkpoint_at(now, instant + std::time::Duration::from_secs(60))?;
        let receipts: u64 = ledger.db.query_row("SELECT count(*) FROM native_slo_minutes_v1", [], |r| r.get(0))?;
        assert_eq!(receipts, 3);
        Ok(())
    }

    #[test]
    fn regular_checkpoint_jitter_preserves_complete_minutes_and_failures_age_out() -> Result<()> {
        let instant = Instant::now();
        let mut ledger = Ledger { db: open(Path::new(":memory:"), "node")?, streams: vec!["x".into()].into(),
            pending: BTreeMap::new(), started_ms: 5, last_ms: 5, last_instant: instant,
            committed_through_ms: 0 };
        for tick in 1..=7 {
            ledger.checkpoint_at(tick * MINUTE + 5,
                instant + std::time::Duration::from_secs(tick * 60))?;
        }
        let receipts: u64 = ledger.db.query_row("SELECT count(*) FROM native_slo_minutes_v1", [], |r| r.get(0))?;
        assert_eq!(receipts, 6); // The startup partial minute alone is excluded.
        let projection = ledger.projection_at(7 * MINUTE + 5)?;
        let active = report_at(Some(&projection), "x", 7 * MINUTE + 5);
        assert_eq!(active["windows"]["5m"]["covered_ms"], 5 * MINUTE);
        let stopped = report_at(Some(&projection), "x", 13 * MINUTE + 5);
        assert_eq!(stopped["available"], false);
        assert_eq!(stopped["coverage"], "unknown");
        assert_eq!(stopped["as_of_unix_ms"], 7 * MINUTE + 5);
        assert_eq!(stopped["through_unix_ms"], 13 * MINUTE);
        assert_eq!(stopped["committed_through_unix_ms"], 7 * MINUTE);
        assert_eq!(stopped["windows"]["5m"]["covered_ms"], 0);
        assert_eq!(stopped["windows"]["5m"]["unknown_ms"], 5 * MINUTE);
        Ok(())
    }

    #[test]
    fn ledger_schema_bounds_node_and_stream_identity_bytes() -> Result<()> {
        assert!(open(Path::new(":memory:"), &"n".repeat(257)).is_err());
        let db = open(Path::new(":memory:"), "node")?;
        assert!(db.execute("INSERT INTO native_slo_minutes_v1 VALUES(?1,0,0,0)", [&"x".repeat(257)]).is_err());
        assert!(db.execute("INSERT INTO native_slo_gauges_v1 VALUES(?1,0,1)", [&"é".repeat(129)]).is_err());
        Ok(())
    }
    #[test]
    fn completion_recording_never_waits_for_blocked_checkpoint_io() -> Result<()> {
        use std::sync::mpsc;
        use std::time::Duration;
        std::thread_local! {
            static BLOCKED: std::cell::RefCell<Option<mpsc::Sender<()>>> =
                const { std::cell::RefCell::new(None) };
        }
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("blocked.sqlite");
        let streams: Arc<[String]> = vec!["latency/300".into()].into();
        let recorder = Arc::new(Recorder { streams: Arc::clone(&streams), pending: Mutex::new(BTreeMap::new()) });
        let now = unix_ms();
        let ledger = Arc::new(Mutex::new(Some(Ledger {
            db: open(&path, "node")?, streams, pending: BTreeMap::new(),
            started_ms: now.saturating_sub(2 * MINUTE), last_ms: now.saturating_sub(MINUTE),
            last_instant: Instant::now() - Duration::from_secs(60),
            committed_through_ms: (now / MINUTE * MINUTE).saturating_sub(MINUTE),
        })));
        recorder.record("latency/300", false);
        let blocker = Connection::open(&path)?;
        blocker.execute_batch("BEGIN EXCLUSIVE")?;
        let (entered, blocked) = mpsc::channel();
        let checkpoint = std::thread::spawn({
            let recorder = Arc::clone(&recorder);
            let ledger = Arc::clone(&ledger);
            move || -> Result<()> {
                BLOCKED.with(|sender| *sender.borrow_mut() = Some(entered));
                ledger.lock().unwrap().as_mut().unwrap().db.busy_handler(Some(|attempt| {
                    if attempt == 0 {
                        BLOCKED.with(|sender| { let _ = sender.borrow().as_ref().unwrap().send(()); });
                    }
                    std::thread::sleep(Duration::from_millis(1));
                    true
                }))?;
                checkpoint_with(&recorder, &ledger, &RwLock::new(None))
            }
        });
        // SQLite itself acknowledges the blocked checkpoint; no scheduling guess or fake I/O.
        let entered_io = blocked.recv_timeout(Duration::from_secs(5));
        let (sent, received) = mpsc::channel();
        let recording = std::thread::spawn({
            let recorder = Arc::clone(&recorder);
            move || { recorder.record("latency/300", true); sent.send(()).unwrap(); }
        });
        let recorded_while_blocked = received.recv_timeout(Duration::from_secs(1));
        blocker.execute_batch("ROLLBACK")?;
        checkpoint.join().unwrap()?;
        recording.join().unwrap();
        assert!(entered_io.is_ok(), "checkpoint never reached its real SQLite wait: {entered_io:?}");
        assert!(recorded_while_blocked.is_ok(), "completion waited for checkpoint I/O");
        let pending = recorder.pending.lock().unwrap();
        assert_eq!(pending.values().map(|counts| counts.eligible).sum::<u64>(), 1);
        assert_eq!(pending.values().map(|counts| counts.bad).sum::<u64>(), 1);
        Ok(())
    }

    #[test]
    fn failed_checkpoint_snapshots_merge_without_losing_new_completions() -> Result<()> {
        let streams: Arc<[String]> = vec!["latency/300".into()].into();
        let recorder = Recorder { streams: Arc::clone(&streams), pending: Mutex::new(BTreeMap::new()) };
        let now = unix_ms();
        let db = open(Path::new(":memory:"), "node")?;
        db.execute_batch("PRAGMA query_only=ON")?;
        let ledger = Mutex::new(Some(Ledger { db, streams, pending: BTreeMap::new(),
            started_ms: now, last_ms: now, last_instant: Instant::now(),
            committed_through_ms: now / MINUTE * MINUTE }));
        let projection = RwLock::new(None);
        recorder.record("latency/300", false);
        assert!(checkpoint_with(&recorder, &ledger, &projection).is_err());
        recorder.record("latency/300", true);
        assert!(checkpoint_with(&recorder, &ledger, &projection).is_err());
        let guard = ledger.lock().unwrap();
        let retained = &guard.as_ref().unwrap().pending;
        assert_eq!(retained.values().map(|counts| counts.eligible).sum::<u64>(), 2);
        assert_eq!(retained.values().map(|counts| counts.bad).sum::<u64>(), 1);
        assert!(recorder.pending.lock().unwrap().is_empty());
        Ok(())
    }
}
