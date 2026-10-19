//! Measure the retired-identity ledger on a file-backed store: tombstone recording with and
//! without the ledger triggers, adoption's keep-only deletion, the import's per-page writer
//! hold, and the ledger's bytes. Usage: retired_ledger_bench [TOMBSTONES] [DIRECTORY]
use smallclaims::Store;
use smallclaims::store::checkpoint::{ClaimTombstone, EnvelopeTombstone, record_checkpoint_tombstones_tx};
use smallclaims::store::checkpoint_trim::keep_only_tombstones_tx;
use smallclaims::store::runtime::Plain;
use std::sync::Arc;
use std::time::{Duration, Instant};

const CHUNK: u64 = 10_000;
const TRIGGERS: [&str; 6] = [
    "retired_claim_recorded_v1",
    "retired_claim_rewritten_v1",
    "retired_claim_withdrawn_v1",
    "retired_envelope_recorded_v1",
    "retired_envelope_rewritten_v1",
    "retired_envelope_withdrawn_v1",
];

fn tombstones(from: u64, to: u64) -> (Vec<EnvelopeTombstone>, Vec<ClaimTombstone>) {
    let envelopes = (from..to)
        .map(|n| EnvelopeTombstone {
            writer: format!("host/member-{}", n % 12),
            sequence: n / 12,
            envelope_hash: format!("{:064x}", n * 7919 + 1),
            accepted_at_unix_ms: 1,
        })
        .collect::<Vec<_>>();
    let claims = envelopes
        .iter()
        .enumerate()
        .map(|(index, envelope)| ClaimTombstone {
            id: format!("{:064x}", (from + index as u64) * 104_729 + 3),
            writer: envelope.writer.clone(),
            sequence: envelope.sequence,
            envelope_hash: envelope.envelope_hash.clone(),
            subject: format!("agent/example/member-{}", index % 50),
            kind: "harness.observed".into(),
            actor: None,
            predecessors: Vec::new(),
            operation_id: None,
            request_digest: None,
            accepted_at_unix_ms: 1,
        })
        .collect();
    (envelopes, claims)
}

fn record(store: &Store, total: u64) -> Duration {
    let started = Instant::now();
    let mut from = 0;
    while from < total {
        let to = (from + CHUNK).min(total);
        let (envelopes, claims) = tombstones(from, to);
        let mut writer = store.connection.write();
        let transaction = writer.transaction().unwrap();
        record_checkpoint_tombstones_tx(&transaction, "checkpoint/bench", &envelopes, &claims)
            .unwrap();
        transaction.commit().unwrap();
        from = to;
    }
    started.elapsed()
}

fn keep_none(store: &Store) -> Duration {
    let started = Instant::now();
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    keep_only_tombstones_tx(&transaction, &[], &[]).unwrap();
    transaction.commit().unwrap();
    started.elapsed()
}

fn ledger_bytes(store: &Store) -> Option<i64> {
    store
        .readers
        .get()
        .query_row(
            "SELECT SUM(pgsize) FROM dbstat WHERE name LIKE 'retired_%'",
            [],
            |row| row.get(0),
        )
        .ok()
}

fn open(directory: &std::path::Path, name: &str) -> Store {
    Store::open(&directory.join(name), "bench", Arc::new(Plain)).unwrap()
}

fn main() {
    let total: u64 = std::env::args().nth(1).map_or(1_000_000, |n| n.parse().unwrap());
    let directory = std::env::args()
        .nth(2)
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from);

    // Recording without the ledger, as an earlier build records it.
    let without = open(&directory, "bench-without.sqlite3");
    for trigger in TRIGGERS {
        without.connection.write().execute_batch(&format!("DROP TRIGGER {trigger}")).unwrap();
    }
    let record_without = record(&without, total);
    // Those rows predate the ledger; the import unions them.
    let mut holds = Vec::new();
    let import_started = Instant::now();
    loop {
        let (page, held) = without.import_retired_identities().unwrap();
        holds.push(held);
        if page.done {
            break;
        }
    }
    let import_work = import_started.elapsed();
    let bytes = ledger_bytes(&without);
    holds.sort();
    let percentile = |p: f64| holds[((holds.len() - 1) as f64 * p).round() as usize];

    // Recording with the ledger triggers, then adoption withdrawing a tenth of it.
    let with = open(&directory, "bench-with.sqlite3");
    let record_with = record(&with, total);
    let subset = (total / 10).max(1);
    let small_without = open(&directory, "bench-small-without.sqlite3");
    for trigger in TRIGGERS {
        small_without.connection.write().execute_batch(&format!("DROP TRIGGER {trigger}")).unwrap();
    }
    record(&small_without, subset);
    let keep_without = keep_none(&small_without);
    let small_with = open(&directory, "bench-small-with.sqlite3");
    record(&small_with, subset);
    let keep_with = keep_none(&small_with);

    println!("tombstones (claims and envelopes each): {total}");
    println!("record without ledger: {record_without:?}; with ledger triggers: {record_with:?} (chunks of {CHUNK})");
    println!(
        "import: {} pages, hold p50 {:?} p99 {:?} max {:?}, total writer work {import_work:?} (rests excluded)",
        holds.len(),
        percentile(0.5),
        percentile(0.99),
        holds[holds.len() - 1]
    );
    match bytes {
        Some(bytes) => println!(
            "ledger bytes (dbstat, both identity tables): {bytes} = {:.1} B per claim+envelope pair",
            bytes as f64 / total as f64
        ),
        None => println!("ledger bytes: dbstat unavailable in this SQLite build"),
    }
    println!("keep-only of {subset} without ledger: {keep_without:?}; with ledger triggers: {keep_with:?}");
    for name in ["bench-without", "bench-with", "bench-small-without", "bench-small-with"] {
        for suffix in ["sqlite3", "sqlite3-wal", "sqlite3-shm"] {
            let _ = std::fs::remove_file(directory.join(format!("{name}.{suffix}")));
        }
    }
}
