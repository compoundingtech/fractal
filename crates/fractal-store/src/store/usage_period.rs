//! Period reads seek the last snapshot at each reporting boundary, not every historical rollup.
use super::*;

pub const INDEX: &str = "claims_usage_period_boundary_index";
pub const PREDICATE: &str = "kind='harness.usage' AND json_extract(body,'$.fields.semantics')='response_rollup'";
const SERIES_FIELDS: [&str; 6] = ["incarnation_id", "model", "account", "owner_run", "owner_step", "host"];

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
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-h46ee499352b7 owner=fractal_store::store::usage_period TODO=chunk-or-reference-variable-columns */ "CREATE INDEX IF NOT EXISTS {INDEX} ON claims({}, {}) WHERE {PREDICATE}",
        series_sql(), observed_sql()
    ))?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h3686c9ce2155 owner=fractal_store::store::usage_period TODO=bound-or-reference-values-and-reclamation */ "DROP INDEX IF EXISTS claims_usage_rollup_index")?;
    Ok(())
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
