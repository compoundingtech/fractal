//! The part of `store::usage_period` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use fractal_delivery::store::usage_period::*;
pub use fractal_delivery::store::usage_period::{INDEX, PREDICATE, create_schema, observed_sql, series_sql};

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
    fn boundary_seeks_use_index_without_history_sort() {
        let store = fixture(48);
        let connection = store.readers.get();
        for sql in [
            format!("SELECT series FROM local_usage_period_v1 INDEXED BY {INDEX} WHERE series>?1 ORDER BY series LIMIT 1"),
            format!("SELECT observed FROM local_usage_period_v1 INDEXED BY {INDEX} WHERE series=?1 AND observed<=?2 ORDER BY observed DESC LIMIT 1"),
            format!("SELECT series FROM local_usage_period_v1 INDEXED BY {WINDOW_INDEX} WHERE observed>?1 AND observed<=?2"),
        ] {
            let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let params = vec![""; statement.parameter_count()];
            let plan = statement.query_map(rusqlite::params_from_iter(params), |row| row.get::<_, String>(3))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
            let index = if sql.contains(WINDOW_INDEX) { WINDOW_INDEX } else { INDEX };
            assert!(plan.contains(&format!("SEARCH local_usage_period_v1 USING COVERING INDEX {index}")), "{plan}");
            assert!(!plan.contains("TEMP B-TREE"), "{plan}");
        }
    }
}
