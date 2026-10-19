//! Authored controls, NOT executed. Raw-column fixtures use a real Store/schema and
//! canonical helpers; they are not signed admission, Source, or public-route evidence.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::message_point_qualification_tests_shared_delivery::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::message_point_qualification_tests_shared_delivery::{batch, claim, compare_core_with_real_oracles, fixture, replica, sent};
use super::*;
use smallclaims::read_budget::{ReadBudget, with};
use smallclaims::sqlite::work::{SqliteWork, SqliteWorkScope};
use std::time::Duration;

#[test]
fn metadata_opcodes_admit_every_variable_column_before_payload() {
    let store = fixture();
    let db = store.readers.get();
    let index_root: i64 = db
        .query_row(
            "SELECT rootpage FROM sqlite_schema WHERE name='claims_subject_index'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let sql = own_headers_sql();
    let mut statement = db.prepare(/* sqlite-large-value: allow-for-now lv-h8a6342079294 owner=st3_store::store::message_point_qualification::tests TODO=trace-statement-and-value-representation */ &format!("EXPLAIN {sql}")).unwrap();
    let mut rows = statement.raw_query();
    let mut roots = BTreeMap::new();
    let mut columns = 0;
    while let Some(row) = rows.next().unwrap() {
        let opcode: String = row.get(1).unwrap();
        let cursor: i64 = row.get(2).unwrap();
        let column: i64 = row.get(3).unwrap();
        if opcode == "OpenRead" {
            roots.insert(cursor, column);
        }
        if opcode == "Column" {
            let flags: u8 = row.get(6).unwrap();
            let integer_position = roots.get(&cursor) == Some(&index_root) && column == 1;
            assert!(
                matches!(flags & 0xc0, 0x80 | 0xc0) || integer_position,
                "unadmitted OP_Column cursor={cursor}, column={column}, flags={flags}"
            );
            columns += 1;
        }
    }
    assert!(columns > 0);
}

