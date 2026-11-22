//! Rebuildable usage lookup, filled in bounded background transactions rather than at open.
use super::*;

pub const INDEX: &str = "local_usage_period_v1_series";
pub const WINDOW_INDEX: &str = "local_usage_period_v1_window";
pub const BACKFILL_BATCH: usize = 64;
pub const PREDICATE: &str = "kind='harness.usage' AND json_extract(body,'$.fields.semantics')='response_rollup'";
const SERIES_FIELDS: [&str; 7] = ["incarnation_id", "model", "account", "owner_run", "owner_step", "host", "execution_id"];

pub fn series_sql() -> String {
    let fields = SERIES_FIELDS.map(|field| format!(
        "CASE WHEN json_type(body,'$.fields.{field}')='text' THEN json_extract(body,'$.fields.{field}') ELSE '' END"
    ));
    format!("json_array(subject,{})", fields.join(","))
}

pub fn observed_sql() -> &'static str {
    // Space-padded decimal text preserves the full u64 domain, unlike SQLite INTEGER/REAL.
    // Non-u64 JSON readings have the same zero fallback as Value::as_u64 in the fold.
    "CASE WHEN json_type(body,'$.fields.observed_at_unix_ms')='integer' \
     AND (body -> '$.fields.observed_at_unix_ms') NOT LIKE '-%' \
     AND length(body -> '$.fields.observed_at_unix_ms')<=20 \
     AND printf('%020s',body -> '$.fields.observed_at_unix_ms')<='18446744073709551615' \
     THEN printf('%020s',body -> '$.fields.observed_at_unix_ms') ELSE printf('%020s','0') END"
}

pub fn create_schema(connection: &Connection) -> Result<()> {
    // Both indexes are installed on an empty, dedicated projection, never on claims.
    // Versioned local state is disposable: claims and the triggers below regenerate it.
    let series = series_sql();
    let observed = observed_sql();
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-h8c2a4192756e owner=fractal_store::store::usage_period TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_usage_period_v1 (
            claim_id TEXT PRIMARY KEY, series TEXT NOT NULL, observed TEXT NOT NULL
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS {INDEX} ON local_usage_period_v1(series,observed);
         CREATE INDEX IF NOT EXISTS {WINDOW_INDEX} ON local_usage_period_v1(observed,series);
         CREATE TABLE IF NOT EXISTS local_usage_period_v1_progress (
            singleton INTEGER PRIMARY KEY CHECK(singleton=1), through_index INTEGER NOT NULL,
            target_index INTEGER NOT NULL
         );
         INSERT OR IGNORE INTO local_usage_period_v1_progress
            SELECT 1,0,COALESCE(MAX(store_index),0) FROM claims;
         CREATE TRIGGER IF NOT EXISTS local_usage_period_v1_insert AFTER INSERT ON claims BEGIN
            INSERT OR REPLACE INTO local_usage_period_v1
              SELECT id,{series},{observed} FROM claims WHERE id=NEW.id AND {PREDICATE};
         END;
         CREATE TRIGGER IF NOT EXISTS local_usage_period_v1_delete AFTER DELETE ON claims BEGIN
            DELETE FROM local_usage_period_v1 WHERE claim_id=OLD.id;
         END;
         CREATE TRIGGER IF NOT EXISTS local_usage_period_v1_update AFTER UPDATE ON claims BEGIN
            DELETE FROM local_usage_period_v1 WHERE claim_id=OLD.id;
            INSERT OR REPLACE INTO local_usage_period_v1
              SELECT id,{series},{observed} FROM claims WHERE id=NEW.id AND {PREDICATE};
         END;"
    ))?;
    Ok(())
}

pub fn ready(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT through_index>=target_index FROM local_usage_period_v1_progress WHERE singleton=1",
        [], |row| row.get(0),
    )?)
}

/// At most 64 source rows, including unrelated claims, per committed writer turn.
pub fn backfill_step_tx(tx: &Transaction<'_>) -> Result<bool> {
    let (through, target): (u64, u64) = tx.query_row(
        "SELECT through_index,target_index FROM local_usage_period_v1_progress WHERE singleton=1",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if through >= target { return Ok(true); }
    let end: u64 = tx.query_row(
        "SELECT COALESCE(MAX(store_index),?2) FROM
         (SELECT store_index FROM claims WHERE store_index>?1 AND store_index<=?2
          ORDER BY store_index LIMIT ?3)",
        params![through, target, BACKFILL_BATCH], |row| row.get(0),
    )?;
    tx.execute(&format!(
        /* sqlite-large-value: allow-for-now lv-h3aca5e04dde4 owner=fractal_store::store::usage_period TODO=prepare-body-and-predecessor-values */ "INSERT OR REPLACE INTO local_usage_period_v1
         SELECT id,{},{} FROM claims WHERE store_index>?1 AND store_index<=?2 AND {PREDICATE}",
        series_sql(), observed_sql(),
    ), params![through, end])?;
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-h2c8754e606af owner=fractal_store::store::usage_period TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_usage_period_v1_progress SET through_index=?1 WHERE singleton=1", [end],
    )?;
    Ok(end >= target)
}


#[cfg(test)]
mod tests {
    #[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::usage_period_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::usage_period_tests_shared_seats::{DAY, fixture};
use super::*;
    

    #[test]
    fn observed_timestamp_index_preserves_u64_and_invalid_value_fallbacks() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute("CREATE TABLE readings(body TEXT)", []).unwrap();
        for value in [json!(0), json!(u64::MAX), json!(-1), json!(1.5), json!("123"), Value::Null] {
            connection.execute("INSERT INTO readings VALUES (?1)", [json!({"fields":{"observed_at_unix_ms":value}}).to_string()]).unwrap();
            let at: String = connection.query_row(&format!("SELECT {} FROM readings ORDER BY rowid DESC LIMIT 1", observed_sql()), [], |row| row.get(0)).unwrap();
            assert_eq!(at, format!("{:>20}", value.as_u64().unwrap_or(0)));
        }
    }
}
