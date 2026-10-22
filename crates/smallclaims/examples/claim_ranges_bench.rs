//! Measure claim range digests on a file-backed store: the queue trigger's cost on admission and
//! on a trim of N claims, the background fold's writer holds, and the bytes of the digests and of
//! the queue before a fold.
//! Usage: claim_ranges_bench [CLAIMS] [DIRECTORY]
use rusqlite::params;
use smallclaims::Store;
use smallclaims::store::runtime::Plain;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PER_BATCH: u64 = 4;
const CHUNK: u64 = 10_000;

fn open(directory: &std::path::Path, name: &str) -> Store {
    let path = directory.join(name);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    Store::open(&path, "bench", Arc::new(Plain)).unwrap()
}

fn id(n: u64) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(n.to_be_bytes()))
}

/// Claims as a peer's envelopes leave them: batches of four. Plain SQL, so only the triggers
/// differ between stores.
fn admit(store: &Store, total: u64) -> Duration {
    let started = Instant::now();
    let mut from = 0;
    while from < total {
        let to = (from + CHUNK).min(total);
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        for n in from..to {
            let batch = n / PER_BATCH;
            if n % PER_BATCH == 0 {
                transaction
                    .execute(
                        "INSERT INTO batches(id, origin, replica_sequence, previous_hash, hash, accepted_at_unix_ms)
                         VALUES (?1, 'host/peer', ?2, NULL, ?1, '1')",
                        params![format!("batch/{batch}"), batch as i64 + 1],
                    )
                    .unwrap();
            }
            transaction
                .prepare_cached(
                    "INSERT INTO claims(id, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms)
                     VALUES (?1, ?2, ?3, 'example.note', 'host/peer', NULL, '{}', '[]', '1')",
                )
                .unwrap()
                .execute(params![id(n), format!("batch/{batch}"), format!("note/{}", n % 1000)])
                .unwrap();
        }
        transaction.commit().unwrap();
        from = to;
    }
    started.elapsed()
}

/// Delete every other claim, CHUNK per transaction, with a cached statement as trims do.
fn trim(store: &Store, total: u64) -> Duration {
    let started = Instant::now();
    let mut from = 0;
    while from < total {
        let to = (from + 2 * CHUNK).min(total);
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        for n in (from..to).step_by(2) {
            transaction.prepare_cached("DELETE FROM claims WHERE id=?1").unwrap().execute([id(n)]).unwrap();
        }
        transaction.commit().unwrap();
        from = to;
    }
    started.elapsed()
}

/// Fold everything, recording each loan's hold.
fn fold(store: &Store) -> Vec<Duration> {
    let mut holds = Vec::new();
    loop {
        let (page, held) = store.fold_claim_ranges().unwrap();
        if page.done {
            return holds;
        }
        holds.push(held);
    }
}

fn bytes(store: &Store, table: &str) -> i64 {
    store
        .readers
        .get()
        .query_row("SELECT COALESCE(SUM(pgsize), 0) FROM dbstat WHERE name=?1", [table], |row| row.get(0))
        .unwrap()
}

fn per_claim(duration: Duration, claims: u64) -> String {
    format!("{duration:.2?} ({:.2} us/claim)", duration.as_secs_f64() * 1e6 / claims as f64)
}

fn holds(mut holds: Vec<Duration>) -> String {
    holds.sort();
    let total: Duration = holds.iter().sum();
    let at = |q: f64| holds[((holds.len() as f64 - 1.0) * q).round() as usize];
    format!(
        "{} loans, hold p50 {:.1?} p99 {:.1?} max {:.1?}, {total:.2?} in total",
        holds.len(),
        at(0.5),
        at(0.99),
        holds[holds.len() - 1]
    )
}

fn main() {
    let total: u64 = std::env::args().nth(1).map_or(200_000, |n| n.parse().unwrap());
    let directory = std::env::args().nth(2).map_or_else(std::env::temp_dir, std::path::PathBuf::from);

    let without = open(&directory, "ranges-without.sqlite3");
    without
        .connection
        .write()
        .execute_batch("DROP TRIGGER claim_range_insert_v1; DROP TRIGGER claim_range_delete_v1;")
        .unwrap();
    let with = open(&directory, "ranges-with.sqlite3");
    let admitted = (admit(&without, total), admit(&with, total));
    let queued = bytes(&with, "claim_range_pending");
    let folded = fold(&with);
    let digests = bytes(&with, "claim_range_digests");
    let trims = (trim(&without, total), trim(&with, total));
    let refolded = fold(&with);
    assert!(with.claim_ranges_complete().unwrap());

    println!("claims: {total} ({PER_BATCH} per batch, {CHUNK} per transaction)");
    println!("admission without triggers: {}", per_claim(admitted.0, total));
    println!("admission with triggers:    {}", per_claim(admitted.1, total));
    println!("queue before the fold: {queued} B ({:.1} B/claim)", queued as f64 / total as f64);
    println!("fold of {total} inserts: {}", holds(folded));
    println!("digests after the fold: {digests} B");
    println!("trim of {} claims without triggers: {}", total / 2, per_claim(trims.0, total / 2));
    println!("trim of {} claims with triggers:    {}", total / 2, per_claim(trims.1, total / 2));
    println!("fold of {} deletes: {}", total / 2, holds(refolded));
}
