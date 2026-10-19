//! How big this member's store is, and how much it grows a day: the database target in
//! `slo/targets.toml`, and the physical growth that disk-filling conditions read. The daemon
//! samples the store hourly and keeps two days of samples in `meta`, so the daily growth survives
//! restarts. A sample reads three pragmas and the WAL file's length. It is the one sampler: the
//! newest sample is cached for every reader (`st3::slo::database_size`).

use super::*;

pub use crate::slo::DatabaseSize;

/// The `meta` key holding recent samples:
/// `[[unix_ms, live_bytes, file_bytes, wal_bytes, main_file_bytes], ...]`, oldest first. Samples
/// written before the files were measured have three elements.
const SAMPLES: &str = "database_size_samples";
/// Samples older than this are forgotten.
const KEEP_MS: u128 = 49 * 60 * 60 * 1000;
/// At most this many samples are kept, whatever the restarts.
const MAX_SAMPLES: usize = 128;
pub const DAY_MS: u128 = 24 * 60 * 60 * 1000;
/// The shortest span a daily growth is extrapolated from.
const MIN_SPAN_MS: u128 = 60 * 60 * 1000;
/// How far from exactly a day ago a physical baseline may be.
const BASELINE_TOLERANCE_MS: u128 = 30 * 60 * 1000;

/// One stored sample: when, the pages in use, the pages in all, and the lengths of the main file
/// and its WAL when they were measured.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    at: u128,
    live: u64,
    file: u64,
    wal: Option<u64>,
    main: Option<u64>,
}

impl Sample {
    fn from_value(value: &Value) -> Option<Self> {
        let item = |index: usize| value.get(index).and_then(Value::as_u64);
        Some(Sample {
            at: u128::from(item(0)?),
            live: item(1)?,
            file: item(2)?,
            wal: item(3),
            main: item(4),
        })
    }

    fn to_value(self) -> Value {
        let mut row = vec![json!(self.at as u64), json!(self.live), json!(self.file)];
        if let (Some(wal), Some(main)) = (self.wal, self.main) {
            row.extend([json!(wal), json!(main)]);
        }
        Value::Array(row)
    }

    /// The main file's length and its WAL's: what the store takes on disk.
    fn physical(self) -> Option<u64> {
        Some(self.main? + self.wal?)
    }
}

/// The daily physical growth at `now` for `physical` bytes, from the sample with a measured WAL
/// nearest exactly a day ago and within half an hour of it, and the span it was measured over.
fn physical_growth(samples: &[Sample], now: u128, physical: u64) -> Option<(i64, u128)> {
    let day_ago = now.checked_sub(DAY_MS)?;
    let (span, then) = samples
        .iter()
        .filter_map(|sample| Some((sample.at, sample.physical()?)))
        .filter(|(at, _)| at.abs_diff(day_ago) <= BASELINE_TOLERANCE_MS)
        .min_by_key(|(at, _)| (at.abs_diff(day_ago), *at))
        .map(|(at, then)| (now - at, then))?;
    let delta = i128::from(physical) - i128::from(then);
    Some((i64::try_from(delta * DAY_MS as i128 / span as i128).ok()?, span))
}

/// The daily growth `samples` show at `now` for `live` bytes, and the span it was measured over.
pub fn growth(samples: &[(u128, u64, u64)], now: u128, live: u64) -> Option<(i64, u128)> {
    let day_ago = now.saturating_sub(DAY_MS);
    // The newest sample at least a day old, or else the oldest sample.
    let (at, then, _) = samples
        .iter()
        .rev()
        .find(|(at, _, _)| *at <= day_ago)
        .or_else(|| samples.first())?;
    let span = now.checked_sub(*at)?;
    if span < MIN_SPAN_MS {
        return None;
    }
    let delta = i128::from(live) - i128::from(*then);
    let per_day = delta * DAY_MS as i128 / span as i128;
    Some((i64::try_from(per_day).ok()?, span))
}

/// The free pages one write gives back to the filesystem: 2 MiB of 4 KiB pages. On a 32 GB
/// store a chunk of 2,048 took 132 ms (p99 226 ms); this keeps each write near 35 ms.
pub const FREE_PAGE_CHUNK: u64 = 512;

impl Store {
    /// Give up to [`FREE_PAGE_CHUNK`] free pages back to the filesystem in one short write, and
    /// return how many went. Only a store with incremental auto-vacuum can: a fresh one, or one
    /// converted by a VACUUM. Any other store's file never shrinks, so this gives back nothing.
    pub fn release_free_pages(&self) -> Result<u64> {
        let connection = self.connection.write_background();
        let free = || -> Result<u64> {
            Ok(connection.query_row("PRAGMA freelist_count", [], |row| row.get(0))?)
        };
        // 2 is INCREMENTAL.
        let mode: u64 = connection.query_row("PRAGMA auto_vacuum", [], |row| row.get(0))?;
        let before = free()?;
        if mode != 2 || before == 0 {
            return Ok(0);
        }
        // FREE_PAGE_CHUNK pages. Each step gives back one, so the pragma runs to its end.
        let mut statement = connection.prepare("PRAGMA incremental_vacuum(512)")?;
        let mut steps = statement.query([])?;
        while steps.next()?.is_some() {}
        drop(steps);
        drop(statement);
        Ok(before.saturating_sub(free()?))
    }

    /// Measure the store and remember the sample. The daemon calls it hourly.
    pub fn record_database_size(&self, now: u128) -> Result<DatabaseSize> {
        let (page_size, pages, free) = {
            let connection = self.readers.get();
            let pragma = |name: &str| -> Result<u64> {
                Ok(connection.query_row(/* sqlite-large-value: allow-for-now lv-h2be7c984aa53 owner=st3_store::store::database_size TODO=trace-statement-and-value-representation */ &format!("PRAGMA {name}"), [], |row| row.get(0))?)
            };
            (pragma("page_size")?, pragma("page_count")?, pragma("freelist_count")?)
        };
        let file_bytes = page_size * pages;
        let live_bytes = page_size * pages.saturating_sub(free);
        // A store without a WAL file beside it (in memory, or just checkpointed away) has none.
        let mut wal = self.path.clone().into_os_string();
        wal.push("-wal");
        let length = |path: &std::ffi::OsStr| std::fs::metadata(path).map_or(0, |metadata| metadata.len());
        let wal_bytes = length(&wal);
        let main_file_bytes = length(self.path.as_os_str());
        let physical_bytes = main_file_bytes + wal_bytes;
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        let mut samples: Vec<Sample> = transaction
            .query_row("SELECT value FROM meta WHERE key=?1", [SAMPLES], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .and_then(|text| serde_json::from_str::<Vec<Value>>(&text).ok())
            .unwrap_or_default()
            .iter()
            .filter_map(Sample::from_value)
            .collect();
        // A sample from the future means the clock went back: it is dropped, never a baseline.
        samples.retain(|sample| sample.at <= now && now - sample.at <= KEEP_MS);
        let triples = samples.iter().map(|sample| (sample.at, sample.live, sample.file)).collect::<Vec<_>>();
        let growth = growth(&triples, now, live_bytes);
        let physical = physical_growth(&samples, now, physical_bytes);
        samples.push(Sample {
            at: now,
            live: live_bytes,
            file: file_bytes,
            wal: Some(wal_bytes),
            main: Some(main_file_bytes),
        });
        let excess = samples.len().saturating_sub(MAX_SAMPLES);
        samples.drain(..excess);
        let stored = Value::Array(samples.iter().map(|sample| sample.to_value()).collect());
        transaction.execute(
            /* sqlite-large-value: allow-for-now lv-h0393cee6d4ec owner=st3_store::store::database_size TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![SAMPLES, stored.to_string()],
        )?;
        transaction.commit()?;
        Ok(DatabaseSize {
            measured_at_unix_ms: now,
            file_bytes,
            live_bytes,
            growth_bytes_per_day: growth.map(|(per_day, _)| per_day),
            growth_span_ms: growth.map(|(_, span)| span),
            wal_bytes,
            main_file_bytes,
            physical_bytes,
            physical_growth_bytes_per_day: physical.map(|(per_day, _)| per_day),
            physical_growth_span_ms: physical.map(|(_, span)| span),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u128 = 60 * 60 * 1000;

    #[test]
    fn growth_is_measured_from_a_day_ago_or_extrapolated_from_an_hour() {
        let now = 100 * DAY_MS;
        assert_eq!(growth(&[], now, 10), None);
        // Less than an hour of history says nothing.
        assert_eq!(growth(&[(now - HOUR / 2, 0, 0)], now, 10), None);
        // Two hours, 100 bytes: 1,200 a day.
        assert_eq!(growth(&[(now - 2 * HOUR, 0, 0)], now, 100), Some((1_200, 2 * HOUR)));
        // With samples on both sides of a day ago, the newest at least a day old.
        let samples = [(now - 30 * HOUR, 0, 0), (now - 25 * HOUR, 500, 0), (now - 3 * HOUR, 900, 0)];
        assert_eq!(growth(&samples, now, 1_500), Some((960, 25 * HOUR)));
        // A trim that frees more than was written is negative growth.
        assert_eq!(growth(&[(now - DAY_MS, 1_000, 0)], now, 400), Some((-600, DAY_MS)));
    }

    fn sample(at: u128, physical: Option<u64>) -> Sample {
        Sample { at, live: 0, file: 0, wal: physical.map(|_| 0), main: physical }
    }

    #[test]
    fn physical_growth_needs_a_baseline_within_half_an_hour_of_a_day_ago() {
        let now = 100 * DAY_MS;
        // No extrapolation: twenty hours of history say nothing.
        assert_eq!(physical_growth(&[sample(now - 20 * HOUR, Some(0))], now, 100), None);
        // A day and twenty minutes ago is close enough, scaled to a day.
        let span = DAY_MS + 20 * 60 * 1000;
        let grown = physical_growth(&[sample(now - span, Some(0))], now, 1_000).unwrap();
        assert_eq!(grown, (i64::try_from(1_000 * DAY_MS / span).unwrap(), span));
        // A day and forty minutes is not.
        assert_eq!(physical_growth(&[sample(now - DAY_MS - 40 * 60 * 1000, Some(0))], now, 1_000), None);
        // The nearest of several, and never one without a measured WAL.
        let samples = [
            sample(now - DAY_MS - 25 * 60 * 1000, Some(100)),
            sample(now - DAY_MS + 5 * 60 * 1000, Some(400)),
            sample(now - DAY_MS, None),
        ];
        assert_eq!(physical_growth(&samples, now, 400).map(|(per_day, _)| per_day), Some(0));
    }

    #[test]
    fn a_sample_from_before_the_wal_was_measured_still_reads() {
        let old = Sample::from_value(&json!([5, 1, 2])).unwrap();
        assert_eq!(old, Sample { at: 5, live: 1, file: 2, wal: None, main: None });
        assert_eq!(old.physical(), None, "never a physical baseline");
        let measured = Sample { at: 5, live: 1, file: 9, wal: Some(3), main: Some(4) };
        assert_eq!(Sample::from_value(&measured.to_value()), Some(measured));
        // The main file and the WAL, not the page count, which includes the WAL's pages.
        assert_eq!(measured.physical(), Some(7));
    }

    #[test]
    fn a_clock_that_went_back_drops_the_future_samples() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("claims.sqlite3"), "alder").unwrap();
        let start = 100 * DAY_MS;
        store.record_database_size(start + DAY_MS).unwrap();
        // The clock goes back a day: the later sample is no baseline and is forgotten.
        let back = store.record_database_size(start).unwrap();
        assert_eq!((back.growth_bytes_per_day, back.physical_growth_bytes_per_day), (None, None));
        let on_disk = |suffix: &str| {
            std::fs::metadata(directory.path().join(format!("claims.sqlite3{suffix}"))).map_or(0, |m| m.len())
        };
        // Saving this sample can append a WAL frame: compare its pre-write measurement.
        let before = (on_disk(""), on_disk("-wal"));
        let next = store.record_database_size(start + DAY_MS).unwrap();
        assert!(next.physical_growth_bytes_per_day.is_some(), "the earlier sample is a day old");
        assert_eq!(next.physical_bytes, next.main_file_bytes + next.wal_bytes);
        assert_eq!((next.main_file_bytes, next.wal_bytes), before);
    }

    fn pragma(store: &Store, name: &str) -> u64 {
        store.readers.get().query_row(&format!("PRAGMA {name}"), [], |row| row.get(0)).unwrap()
    }

    /// The file's pages less its pointer maps. An incremental auto-vacuum file keeps a pointer
    /// map at page 2 and then every `page_size / 5 + 1` pages, and drops any that a truncation
    /// leaves past the end, so the file can shrink by one page more than it gave back.
    fn content_pages(store: &Store) -> u64 {
        let pages = pragma(store, "page_count");
        let mapped = pragma(store, "page_size") / 5 + 1;
        pages - if pages < 2 { 0 } else { (pages - 2) / mapped + 1 }
    }

    /// Delete rows the way a trim does, leaving their pages free; return how many.
    fn free_some_pages(store: &Store) -> u64 {
        store
            .connection
            .write()
            .execute_batch(
                "CREATE TABLE trimmed(value BLOB);
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 600)
                 INSERT INTO trimmed SELECT zeroblob(8000) FROM n;
                 DROP TABLE trimmed;",
            )
            .unwrap();
        pragma(store, "freelist_count")
    }

    #[test]
    fn a_new_store_gives_freed_pages_back_a_chunk_at_a_time() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let store = Store::open(&path, "alder").unwrap();
        assert_eq!(pragma(&store, "auto_vacuum"), 2, "incremental");
        let free = free_some_pages(&store);
        let pages = content_pages(&store);
        assert!(free > 1_000, "{free} free pages");
        assert_eq!(store.release_free_pages().unwrap(), FREE_PAGE_CHUNK);
        assert_eq!(
            (content_pages(&store), pragma(&store, "freelist_count")),
            (pages - FREE_PAGE_CHUNK, free - FREE_PAGE_CHUNK)
        );
        let mut released = FREE_PAGE_CHUNK;
        loop {
            match store.release_free_pages().unwrap() {
                0 => break,
                pages => released += pages,
            }
        }
        assert_eq!(released, free);
        assert_eq!(pragma(&store, "freelist_count"), 0);
        assert_eq!(content_pages(&store), pages - free);
        drop(store);

        // A store from before incremental auto-vacuum keeps its free pages. One VACUUM, run
        // with the daemon stopped, converts it.
        let vacuum = |mode: &str| {
            rusqlite::Connection::open(&path)
                .unwrap()
                .execute_batch(&format!("PRAGMA auto_vacuum = {mode}; VACUUM;"))
                .unwrap();
        };
        vacuum("NONE");
        let store = Store::open(&path, "alder").unwrap();
        assert_eq!(pragma(&store, "auto_vacuum"), 0, "reopening keeps the file's setting");
        assert!(free_some_pages(&store) > 1_000);
        assert_eq!(store.release_free_pages().unwrap(), 0);
        drop(store);
        vacuum("INCREMENTAL");
        let store = Store::open(&path, "alder").unwrap();
        assert_eq!(pragma(&store, "auto_vacuum"), 2);
        assert!(free_some_pages(&store) > 1_000);
        assert_eq!(store.release_free_pages().unwrap(), FREE_PAGE_CHUNK);
    }

    #[test]
    fn samples_survive_reopening_and_age_out() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let start = 100 * DAY_MS;
        {
            let store = Store::open(&path, "alder").unwrap();
            let first = store.record_database_size(start).unwrap();
            assert!(first.live_bytes > 0 && first.file_bytes >= first.live_bytes);
            assert_eq!(first.growth_bytes_per_day, None);
        }
        let store = Store::open(&path, "alder").unwrap();
        let later = store.record_database_size(start + 2 * HOUR).unwrap();
        assert_eq!(later.growth_span_ms, Some(2 * HOUR));
        assert!(later.growth_bytes_per_day.is_some());
        // Three days later the old samples are gone, so there is no growth to report yet.
        let much_later = store.record_database_size(start + 3 * DAY_MS).unwrap();
        assert_eq!(much_later.growth_bytes_per_day, None);
    }
}
