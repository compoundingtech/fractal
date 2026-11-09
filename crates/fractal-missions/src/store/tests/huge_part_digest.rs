//! Test-only deterministic logical-row digest shared by the huge and offline benches.
//! No writes. Identical hashes require identical source claims/identities, not separately
//! generated fixtures. Typed length-delimited values prevent concatenation ambiguity.
use rusqlite::{Connection, params, types::ValueRef};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

fn bytes(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

fn value(hash: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hash.update([0]),
        ValueRef::Integer(value) => { hash.update([1]); hash.update(value.to_be_bytes()); }
        ValueRef::Real(value) => { hash.update([2]); hash.update(value.to_bits().to_be_bytes()); }
        ValueRef::Text(value) => { hash.update([3]); bytes(hash, value); }
        ValueRef::Blob(value) => { hash.update([4]); bytes(hash, value); }
    }
}

/// `subjects` is the rebuild's run/generation/proposal subject set (explicit step
/// subjects may also be present). Generation ranges include all their step claims.
/// Returns the combined digest so callers can assert repeat stability if desired.
pub(super) fn print_part_digest(
    connection: &Connection,
    runs: &BTreeSet<String>,
    subjects: &BTreeSet<String>,
) -> String {
    let mut claim_ids = BTreeSet::new();
    let mut statement = connection.prepare_cached(
        "SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 AND NOT EXISTS(SELECT 1 FROM replica_records
         WHERE replica_records.claim_id=claims.id AND replica_records.state='repaired')",
    ).unwrap();
    for subject in subjects {
        claim_ids.extend(statement.query_map([subject], |row| row.get::<_, String>(0))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap());
    }
    let mut statement = connection.prepare_cached(
        "SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject>=?1 AND claims.subject<?2 AND NOT EXISTS(SELECT 1 FROM replica_records
         WHERE replica_records.claim_id=claims.id AND replica_records.state='repaired')",
    ).unwrap();
    for generation in subjects.iter().filter_map(|subject| subject.strip_prefix("run-generation/")) {
        claim_ids.extend(statement.query_map(params![format!("step-run/{generation}/"),
            format!("step-run/{generation}0")], |row| row.get::<_, String>(0))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap());
    }
    let mut combined = Sha256::new();
    let run_members = serde_json::to_string(runs).unwrap();
    let health_members = serde_json::to_string(&claim_ids.iter()
        .map(|id| format!("projection:runs:{id}")).collect::<Vec<_>>()).unwrap();
    let mut total_rows = 0_u64;
    println!("real_part_digest_format version=1 values=sqlite_typed_length_delimited order=primary_key source_identity_normalization=none");
    println!("real_part_digest_quarantine storage=projection_health prefix=projection:runs: separate_quarantine_table=false");
    for table in ["mission_runs", "run_generations", "step_runs", "revision_proposals",
        "mission_run_deadlines", "mission_run_after", "projection_health"] {
        // Introspection selects every column and discovers composite PK order rather
        // than hard-coding a schema that might silently omit a new projection field.
        let mut schema = connection.prepare(/* sqlite-large-value: allow-for-now lv-h60604afd746c owner=fractal_missions::store::tests::huge_part_digest TODO=trace-statement-and-value-representation */ &format!("PRAGMA table_info({table})")).unwrap();
        let columns = schema.query_map([], |row| Ok((row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, i64>(5)?)))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert!(!columns.is_empty(), "missing digest table {table}");
        let mut primary = columns.iter().filter(|column| column.2 > 0).collect::<Vec<_>>();
        primary.sort_by_key(|column| column.2);
        assert!(!primary.is_empty(), "digest requires primary key for {table}");
        let order = primary.iter().map(|column| format!("\"{}\"", column.0)).collect::<Vec<_>>().join(",");
        let excluded = columns.iter().enumerate().filter_map(|(index, column)| {
            // Quarantine failures stamp now_ms(). Projection row creation/update times
            // instead come from claims and MUST remain covered by the digest.
            (table == "projection_health" && column.0 == "updated_at_unix_ms").then_some(index)
        }).collect::<BTreeSet<_>>();
        let excluded_names = excluded.iter().map(|index| columns[*index].0.as_str()).collect::<Vec<_>>();
        println!("real_part_digest_excluded table={table} columns={} reason=projection_wall_clock", excluded_names.join(","));
        let mut hash = Sha256::new();
        bytes(&mut hash, table.as_bytes());
        for (index, column) in columns.iter().enumerate() {
            if !excluded.contains(&index) {
                bytes(&mut hash, column.0.as_bytes());
                bytes(&mut hash, column.1.as_bytes());
                hash.update(column.2.to_be_bytes());
            }
        }
        let membership = if table == "mission_runs" { "id" }
            else if table == "projection_health" { "aggregate" } else { "run_id" };
        // One bound JSON array avoids SQLite's parameter-count limit for huge parts,
        // while retaining indexed membership lookup rather than scanning unrelated rows.
        let mut query = connection.prepare(/* sqlite-large-value: allow-for-now lv-hd51a8ab00d39 owner=fractal_missions::store::tests::huge_part_digest TODO=trace-statement-and-value-representation */ &format!(
            "SELECT * FROM {table} WHERE {membership} IN (SELECT value FROM json_each(?1)) ORDER BY {order}",
        )).unwrap();
        let members = if table == "projection_health" { &health_members } else { &run_members };
        let mut rows = query.query([members]).unwrap();
        let mut count = 0_u64;
        while let Some(row) = rows.next().unwrap() {
            hash.update([0xff]); // Row boundary, including rows whose values are all NULL.
            for index in 0..columns.len() {
                if !excluded.contains(&index) { value(&mut hash, row.get_ref(index).unwrap()); }
            }
            count += 1;
        }
        hash.update(count.to_be_bytes());
        let digest = hash.finalize();
        println!("real_part_digest table={table} rows={count} sha256={}", hex::encode(digest));
        bytes(&mut combined, table.as_bytes());
        combined.update(count.to_be_bytes());
        combined.update(digest);
        total_rows += count;
    }
    let combined = hex::encode(combined.finalize());
    println!("real_part_digest_combined tables=7 rows={total_rows} claims={} sha256={combined}", claim_ids.len());
    combined
}

// Used by fractal-store's real_part_rebuild_bench, which includes this file by path.
#[allow(dead_code)]
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Stream every non-internal table in this caller's transaction. This deliberately
/// includes caches, digest-maintenance rows, local state and metadata: unexplained
/// differences there must not be hidden by a projection-only equivalence check.
// Used by fractal-store's real_part_rebuild_bench, which includes this file by path.
#[allow(dead_code)]
pub(super) fn print_whole_store_digest(connection: &Connection, phase: &str) -> String {
    let started = std::time::Instant::now();
    let mut progress = started;
    let mut combined = Sha256::new();
    let mut total_rows = 0_u64;
    let mut table_count = 0_u64;
    println!("real_part_whole_digest_format phase={phase} version=1 values=sqlite_typed_length_delimited order=primary_key_or_rowid streaming=true");
    let mut table_query = connection.prepare(
        "SELECT name,sql FROM sqlite_master WHERE type='table' AND substr(name,1,7)<>'sqlite_' ORDER BY name",
    ).unwrap();
    let mut tables = table_query.query([]).unwrap();
    while let Some(table_row) = tables.next().unwrap() {
        let table: String = table_row.get(0).unwrap();
        let definition: String = table_row.get(1).unwrap();
        let quoted = quote_identifier(&table);
        let mut schema = connection.prepare(/* sqlite-large-value: allow-for-now lv-hb8afecd653a5 owner=fractal_missions::store::tests::huge_part_digest TODO=trace-statement-and-value-representation */ &format!("PRAGMA table_info({quoted})")).unwrap();
        // Only column metadata is collected; data rows and their large cells are streamed.
        let columns = schema.query_map([], |row| Ok((row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, i64>(5)?)))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert!(!columns.is_empty(), "missing columns for whole-store table {table}");
        let mut primary = columns.iter().filter(|column| column.2 > 0).collect::<Vec<_>>();
        primary.sort_by_key(|column| column.2);
        let order = if primary.is_empty() {
            assert!(!definition.to_ascii_uppercase().contains("WITHOUT ROWID"),
                "WITHOUT ROWID table {table} must declare its primary key");
            ["rowid", "_rowid_", "oid"].into_iter()
                .find(|alias| !columns.iter().any(|column| column.0.eq_ignore_ascii_case(alias)))
                .expect("all implicit rowid aliases are shadowed").to_owned()
        } else {
            primary.iter().map(|column| quote_identifier(&column.0)).collect::<Vec<_>>().join(",")
        };
        let excluded = columns.iter().enumerate().filter_map(|(index, column)| {
            let reason = match (table.as_str(), column.0.as_str()) {
                ("projection_health", "updated_at_unix_ms") =>
                    Some("project_claim_isolated_tx stamps quarantine failures with now_ms"),
                ("local_observations", "observed_at_unix_ms") =>
                    Some("note_run_projection_finished_tx stamps local completion observations with now_ms"),
                _ => None,
            };
            if let Some(reason) = reason {
                println!("real_part_whole_digest_excluded phase={phase} table={table:?} column={:?} reason={reason:?}", column.0);
                Some(index)
            } else { None }
        }).collect::<BTreeSet<_>>();
        let mut hash = Sha256::new();
        bytes(&mut hash, table.as_bytes());
        for (index, column) in columns.iter().enumerate() {
            if !excluded.contains(&index) {
                bytes(&mut hash, column.0.as_bytes());
                bytes(&mut hash, column.1.as_bytes());
                hash.update(column.2.to_be_bytes());
            }
        }
        println!("real_part_whole_digest_start phase={phase} table={table:?} elapsed_ms={}", started.elapsed().as_millis());
        let mut query = connection.prepare(/* sqlite-large-value: allow-for-now lv-h4596258c4e6d owner=fractal_missions::store::tests::huge_part_digest TODO=trace-statement-and-value-representation */ &format!("SELECT * FROM {quoted} ORDER BY {order}")).unwrap();
        let mut rows = query.query([]).unwrap();
        let mut count = 0_u64;
        while let Some(row) = rows.next().unwrap() {
            hash.update([0xff]);
            for index in 0..columns.len() {
                if !excluded.contains(&index) { value(&mut hash, row.get_ref(index).unwrap()); }
            }
            count += 1;
            if count.is_multiple_of(4096) && progress.elapsed() >= std::time::Duration::from_secs(30) {
                println!("real_part_whole_digest_progress phase={phase} table={table:?} rows={count} elapsed_ms={}",
                    started.elapsed().as_millis());
                progress = std::time::Instant::now();
            }
        }
        hash.update(count.to_be_bytes());
        let digest = hash.finalize();
        println!("real_part_whole_digest phase={phase} table={table:?} rows={count} sha256={}", hex::encode(digest));
        bytes(&mut combined, table.as_bytes());
        combined.update(count.to_be_bytes());
        combined.update(digest);
        total_rows += count;
        table_count += 1;
    }
    let combined = hex::encode(combined.finalize());
    println!("real_part_whole_digest_combined phase={phase} tables={table_count} rows={total_rows} sha256={combined}");
    println!("real_part_whole_digest_time phase={phase} elapsed_ms={}", started.elapsed().as_millis());
    combined
}
