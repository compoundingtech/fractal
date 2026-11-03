//! Measure stored claim positions on a file-backed store: bytes per claim, the admission-time
//! position write, the record-insert trigger, and the delete-side triggers on a trim of N claims
//! (positions stored, and a legacy store whose positions the triggers must freeze).
//! Usage: claim_positions_bench [CLAIMS] [DIRECTORY]
use rusqlite::params;
use smallclaims::Store;
use smallclaims::store::canonical;
use smallclaims::store::runtime::Plain;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PER_BATCH: u64 = 4;
const CHUNK: u64 = 10_000;
const TRIGGERS: [&str; 5] = [
    "claim_positions_record_insert_v1",
    "claim_positions_record_change_v1",
    "claim_positions_record_changed_v1",
    "claim_positions_record_delete_v1",
    "claim_positions_sibling_delete_v1",
];

fn open(directory: &std::path::Path, name: &str) -> Store {
    Store::open(&directory.join(name), "bench", Arc::new(Plain)).unwrap()
}

fn drop_triggers(store: &Store) {
    for trigger in TRIGGERS {
        store.connection.write().execute_batch(&format!("DROP TRIGGER {trigger}")).unwrap();
    }
}

/// Claims admitted as a peer's envelopes would leave them: a batch of four claims and one wire
/// record each. Plain SQL, so only the triggers (not admission code) differ between stores.
fn admit(store: &Store, total: u64, explicit_positions: bool) -> Duration {
    let started = Instant::now();
    let mut from = 0;
    while from < total {
        let to = (from + CHUNK).min(total);
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        for n in from..to {
            let batch = n / PER_BATCH;
            let position = n % PER_BATCH;
            let id = format!("{:064x}", n * 104_729 + 3);
            if position == 0 {
                transaction
                    .execute(
                        "INSERT INTO batches(id, origin, replica_sequence, previous_hash, hash, accepted_at_unix_ms)
                         VALUES (?1, 'host/peer', ?2, NULL, ?1, '1')",
                        params![format!("batch/{batch}"), batch as i64 + 1],
                    )
                    .unwrap();
            }
            if explicit_positions {
                canonical::record_position_tx(&transaction, &id, position).unwrap();
            }
            transaction
                .execute(
                    "INSERT INTO claims(id, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms)
                     VALUES (?1, ?2, ?3, 'example.note', 'host/peer', NULL, '{}', '[]', '1')",
                    params![id, format!("batch/{batch}"), format!("note/{}", n % 1000)],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO replica_records(record_ref, writer, sequence, envelope_hash, position, raw, state, claim_id, updated_at_unix_ms)
                     VALUES (?1, 'host/peer', ?2, ?3, ?4, X'00', 'valid', ?5, '1')",
                    params![format!("record/{n}"), batch as i64 + 1, format!("{batch:064x}"), position as i64, id],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        from = to;
    }
    started.elapsed()
}

/// Delete every claim and record, a chunk of whole batches per transaction.
fn trim(store: &Store, total: u64, cached: bool) -> Duration {
    let started = Instant::now();
    let mut from = 0;
    while from < total {
        let to = (from + CHUNK).min(total);
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        // The order delete_dropped_rows_tx uses: claims, then their envelope's records.
        for n in from..to {
            let id = format!("{:064x}", n * 104_729 + 3);
            if cached {
                transaction.prepare_cached("DELETE FROM claims WHERE id=?1").unwrap().execute([id]).unwrap();
            } else {
                transaction.execute("DELETE FROM claims WHERE id=?1", [id]).unwrap();
            }
        }
        for n in from..to {
            let record = format!("record/{n}");
            if cached {
                transaction.prepare_cached("DELETE FROM replica_records WHERE record_ref=?1").unwrap().execute([record]).unwrap();
            } else {
                transaction.execute("DELETE FROM replica_records WHERE record_ref=?1", [record]).unwrap();
            }
        }
        transaction.commit().unwrap();
        from = to;
    }
    started.elapsed()
}

fn per_claim(duration: Duration, total: u64) -> String {
    format!("{duration:?} ({:.1} us/claim)", duration.as_secs_f64() * 1e6 / total as f64)
}

fn main() {
    let total: u64 = std::env::args().nth(1).map_or(200_000, |n| n.parse().unwrap());
    let directory = std::env::args()
        .nth(2)
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from);

    let mut results = Vec::new();
    let mut bytes: Option<i64> = None;
    let mut frozen = 0;
    let mut admitted = (Duration::ZERO, Duration::ZERO);
    for cached in [false, true] {
        let suffix = if cached { "cached" } else { "uncached" };
        let without = open(&directory, &format!("positions-without-{suffix}.sqlite3"));
        drop_triggers(&without);
        let admit_without = admit(&without, total, false);
        let with = open(&directory, &format!("positions-with-{suffix}.sqlite3"));
        let admit_with = admit(&with, total, true);
        if !cached {
            admitted = (admit_without, admit_with);
            bytes = with
                .readers
                .get()
                .query_row("SELECT SUM(pgsize) FROM dbstat WHERE name='claim_positions'", [], |row| row.get(0))
                .ok();
        }
        let legacy = open(&directory, &format!("positions-legacy-{suffix}.sqlite3"));
        admit(&legacy, total, false);
        legacy.connection.write().execute("DELETE FROM claim_positions", []).unwrap();
        let trims = (trim(&without, total, cached), trim(&with, total, cached), trim(&legacy, total, cached));
        frozen = legacy
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM claim_positions", [], |row| row.get(0))
            .unwrap();
        results.push((suffix, trims));
    }

    println!("claims: {total} ({PER_BATCH} per batch, one wire record each, {CHUNK} per transaction)");
    println!("admit without position triggers or writes: {}", per_claim(admitted.0, total));
    println!("admit with explicit position write and record trigger: {}", per_claim(admitted.1, total));
    match bytes {
        Some(bytes) => println!(
            "claim_positions pages: {bytes} B = {:.1} B per claim",
            bytes as f64 / total as f64
        ),
        None => println!("claim_positions bytes: dbstat unavailable"),
    }
    for (suffix, (without, with, legacy)) in results {
        println!("trim ({suffix} DELETE statements) without triggers: {}", per_claim(without, total));
        println!("trim ({suffix}) with triggers, positions stored: {}", per_claim(with, total));
        println!("trim ({suffix}) with triggers, legacy store (freezes {frozen} positions): {}", per_claim(legacy, total));
    }
    for name in ["positions-without", "positions-with", "positions-legacy"] {
        for variant in ["uncached", "cached"] {
            for suffix in ["sqlite3", "sqlite3-wal", "sqlite3-shm"] {
                let _ = std::fs::remove_file(directory.join(format!("{name}-{variant}.{suffix}")));
            }
        }
    }
}
