//! The part of `store::usage_period` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_attention::store::usage_period::*;
pub use st3_attention::store::usage_period::{create_schema};

pub fn boundary_rows(
    connection: &Connection,
    boundaries: &BTreeSet<u64>,
) -> Result<Vec<(String, u64, String)>> {
    let series = series_sql();
    let observed = observed_sql();
    let boundaries = boundaries.iter().map(|boundary| format!("{boundary:>20}")).collect::<Vec<_>>();
    // A loose index scan: one successor seek per series, regardless of its history length.
    let mut next = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h88cbee2c7e36 owner=st3_seats::store::usage_period TODO=trace-statement-and-value-representation */ &format!(
        "SELECT {series} FROM claims INDEXED BY {INDEX} WHERE {PREDICATE} AND {series}>?1 ORDER BY {series} LIMIT 1"
    ))?;
    let mut timestamp = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hd294ae49c5a0 owner=st3_seats::store::usage_period TODO=trace-statement-and-value-representation */ &format!(
        "SELECT {observed} FROM claims INDEXED BY {INDEX} WHERE {PREDICATE} AND {series}=?1 AND {observed}<=?2 ORDER BY {observed} DESC LIMIT 1"
    ))?;
    let mut snapshots = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hf5e4933479cd owner=st3_seats::store::usage_period TODO=trace-statement-and-value-representation */ &format!(
        "SELECT id,subject,store_index,body FROM claims INDEXED BY {INDEX} WHERE {PREDICATE} AND {series}=?1 AND {observed}=?2 LIMIT 2"
    ))?;
    // Canonical order matters only when a series has several snapshots at the same observed
    // timestamp. Keep the exact replicated tie-breaker, but never sort its whole history.
    let mut tied = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hbc4cdc2788ac owner=st3_seats::store::usage_period TODO=trace-statement-and-value-representation */ &canonical_sql(&format!(
        "SELECT id,subject,store_index,body FROM claims INDEXED BY {INDEX} WHERE {PREDICATE} AND {series}=?1 AND {observed}=?2 ORDER BY CANONICAL_DESC(claims) LIMIT 1"
    )))?;
    let read_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(String, String, u64, String)> {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    };
    let mut result = BTreeMap::new();
    let mut previous = String::new();
    while let Some(key) = next.query_row(/* sqlite-large-value: allow-for-now lv-h1d0def8a68f0 owner=st3_seats::store::usage_period TODO=trace-statement-and-value-representation */ [&previous], |row| row.get::<_, String>(0)).optional()? {
        let mut previous_at = None;
        for boundary in &boundaries {
            let Some(at) = timestamp.query_row(/* sqlite-large-value: allow-for-now lv-h45f90dfcf5db owner=st3_seats::store::usage_period TODO=trace-statement-and-value-representation */ params![key, boundary], |row| row.get::<_, String>(0)).optional()? else {
                continue;
            };
            if previous_at.as_ref() == Some(&at) {
                continue;
            }
            let mut candidates = snapshots.query(params![key, at])?;
            let first = read_row(candidates.next()?.context("indexed usage timestamp has a snapshot")?)?;
            let has_ties = candidates.next()?.is_some();
            drop(candidates);
            let winner = if has_ties {
                tied.query_row(/* sqlite-large-value: allow-for-now lv-hd618852271c1 owner=st3_seats::store::usage_period TODO=trace-statement-and-value-representation */ params![key, at], read_row)?
            } else {
                first
            };
            let (id, subject, index, body) = winner;
            result.insert(id, (subject, index, body));
            previous_at = Some(at);
        }
        previous = key;
    }
    Ok(result.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use st3_attention::store::usage_period_tests_shared_seats::*;
    use st3_attention::store::usage_period_tests_shared_seats::{DAY, fixture};

    fn oracle(store: &Store, since: u64, until: u64) -> (Vec<Value>, Value) {
        let connection = store.readers.get();
        let mut statement = connection.prepare(&canonical_sql(
            "SELECT subject,store_index,body FROM claims INDEXED BY claims_usage_rollup_index
             WHERE kind='harness.usage' AND json_extract(body,'$.fields.semantics')='response_rollup'
             ORDER BY CANONICAL_ASC(claims)",
        )).unwrap();
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        let (rows, days) = Store::usage_period_data_from_rows(rows, since, until).unwrap();
        (rows, store.agent_message_estimate(&days).unwrap())
    }

    #[test]
    fn indexed_boundaries_match_full_fold_with_ties_restarts_and_daily_windows() {
        let store = fixture(12_000);
        for (since, until) in [
            (0, 0), (0, DAY), (DAY, 2 * DAY), (DAY + 1, 3 * DAY - 1),
            (2 * DAY, 3 * DAY), (0, 40 * DAY), (4 * DAY, 5 * DAY),
        ] {
            assert_eq!(store.usage_period_report(since, until).unwrap(), oracle(&store, since, until));
        }
        let boundaries = BTreeSet::from([DAY, 2 * DAY, 3 * DAY]);
        let rows = boundary_rows(&store.readers.get(), &boundaries).unwrap();
        assert!(rows.len() <= 24 * boundaries.len());
        let start = Instant::now();
        let before = oracle(&store, DAY, 2 * DAY);
        let before_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let after = store.usage_period_report(DAY, 2 * DAY).unwrap();
        let after_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(before, after);
        println!("client usage fixture: 12000 rollups / 24 series; full fold {before_ms:.3} ms -> indexed boundaries {after_ms:.3} ms; {} selected snapshots", rows.len());
    }
}
