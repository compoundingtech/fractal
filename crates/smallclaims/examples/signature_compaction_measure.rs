//! Measure the version 26 compaction writer on a copy of a real member's database: claims
//! compacted, each writer loan's hold, bytes per table per signed claim before and after, pages
//! given back, and the replication inventory digest before and after (the compacted tables are
//! outside it). The settled-judging prune of #6875 runs first, as the release before this one
//! leaves the store. Usage: signature_compaction_measure COPY ORIGIN. Run it on a copy only.
use smallclaims::Store;
use smallclaims::store::runtime::Plain;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TABLES: [&str; 8] = [
    "claim_signature_candidates",
    "claim_signature_sources",
    "claim_signature_counts",
    "claim_signature_writer_counts",
    "claim_signatures",
    "claim_signature_nonce_uses",
    "claim_verdicts",
    "claim_signature_judging",
];

struct Holds(Vec<Duration>, Option<rusqlite::Connection>);

impl Holds {
    fn time<T>(&mut self, work: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = work();
        self.0.push(started.elapsed());
        // As the daemon's checkpointer does beside the writer: keep the WAL short, untimed.
        if self.0.len() % 64 == 0
            && let Some(checkpoint) = &self.1
        {
            let _ = checkpoint.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
        }
        result
    }
    fn report(&mut self, name: &str) {
        let slow = self.0.iter().filter(|hold| hold.as_millis() >= 100).count();
        self.0.sort();
        let total: Duration = self.0.iter().sum();
        let at = |q: f64| self.0.get(((self.0.len() as f64 - 1.0) * q) as usize).copied().unwrap_or_default();
        println!(
            "{name}: loans={} at_least_100ms={slow} total_s={:.1} p50_ms={:.2} p99_ms={:.2} max_ms={:.2}",
            self.0.len(),
            total.as_secs_f64(),
            at(0.5).as_secs_f64() * 1e3,
            at(0.99).as_secs_f64() * 1e3,
            at(1.0).as_secs_f64() * 1e3,
        );
    }
}

fn pragma(store: &Store, name: &str) -> u64 {
    store.readers.get().query_row(&format!("PRAGMA {name}"), [], |row| row.get(0)).unwrap()
}

/// Bytes of each table with its indexes, walking only those b-trees.
fn sizes(store: &Store) -> BTreeMap<&'static str, u64> {
    let connection = store.readers.get();
    TABLES
        .iter()
        .map(|table| {
            let bytes = connection
                .query_row(
                    "SELECT COALESCE(SUM(d.pgsize),0) FROM sqlite_schema s, dbstat d
                      WHERE s.tbl_name=?1 AND s.type IN ('table','index') AND d.name=s.name",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            (*table, bytes)
        })
        .collect()
}

/// As `release_free_pages` does: 128 free pages per write. Returns the pages given back.
fn release(store: &Store) -> u64 {
    if pragma(store, "auto_vacuum") != 2 {
        return 0;
    }
    let connection = store.connection.write_background();
    let free = |connection: &rusqlite::Connection| -> u64 {
        connection.query_row("PRAGMA freelist_count", [], |row| row.get(0)).unwrap()
    };
    let before = free(&connection);
    let mut statement = connection.prepare("PRAGMA incremental_vacuum(128)").unwrap();
    let mut steps = statement.query([]).unwrap();
    while steps.next().unwrap().is_some() {}
    drop(steps);
    drop(statement);
    before - free(&connection)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("COPY");
    let origin = args.next().expect("ORIGIN");
    let opened = Instant::now();
    let store = Store::open(std::path::Path::new(&path), &origin, Arc::new(Plain)).unwrap();
    println!("open_ms={}", opened.elapsed().as_millis());
    println!("migration_done={}", store.signature_migration().unwrap().done());
    let digest_before = store.replication_inventory().unwrap().digest;

    let mut holds = Holds(Vec::new(), rusqlite::Connection::open(&path).ok());
    while !holds.time(|| store.prune_settled_signature_judgments().unwrap()).finished {}
    holds.report("judging_prune_first");
    let mut holds = Holds(Vec::new(), rusqlite::Connection::open(&path).ok());
    loop {
        let pages = holds.time(|| release(&store));
        if pages == 0 {
            break;
        }
    }
    holds.report("release_after_prune");

    let signed: u64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM claim_signatures", [], |row| row.get(0))
        .unwrap();
    let file_before = pragma(&store, "page_count") * pragma(&store, "page_size");
    let before = sizes(&store);
    println!("signed={signed} user_version_before={}", pragma(&store, "user_version"));

    let mut holds = Holds(Vec::new(), rusqlite::Connection::open(&path).ok());
    let mut writer = Holds(Vec::new(), None);
    let (mut examined, mut compacted) = (0, 0);
    loop {
        let page = holds.time(|| store.compact_settled_signatures().unwrap());
        let held = page.held;
        writer.0.push(held);
        let wall = *holds.0.last().unwrap();
        if wall.as_millis() >= 100 || held.as_millis() >= 100 {
            println!(
                "slow compaction loan #{}: wall {} ms, writer hold {} ms, examined={} compacted={}",
                holds.0.len() - 1, wall.as_millis(), held.as_millis(), page.examined, page.compacted
            );
        }
        examined += page.examined;
        compacted += page.compacted;
        if page.finished || page.examined == 0 {
            break;
        }
    }
    println!(
        "examined={examined} compacted={compacted} ({:.2}% of signed) user_version_after={}",
        100.0 * compacted as f64 / signed.max(1) as f64,
        pragma(&store, "user_version")
    );
    holds.report("compaction_wall");
    writer.report("compaction_writer_hold");

    let free = pragma(&store, "freelist_count");
    let mut holds = Holds(Vec::new(), rusqlite::Connection::open(&path).ok());
    loop {
        let pages = holds.time(|| release(&store));
        if pages == 0 {
            break;
        }
    }
    println!("free_after_compaction={free}");
    holds.report("release_free_pages");

    let after = sizes(&store);
    for table in TABLES {
        println!(
            "{table:32} before {:6.0} B/signed  after {:6.0} B/signed",
            before[table] as f64 / signed.max(1) as f64,
            after[table] as f64 / signed.max(1) as f64
        );
    }
    let file_after = pragma(&store, "page_count") * pragma(&store, "page_size");
    println!(
        "file: {:.2}% smaller ({:.0} B per signed claim)",
        100.0 * (file_before - file_after) as f64 / file_before as f64,
        (file_before - file_after) as f64 / signed.max(1) as f64
    );
    let digest_after = store.replication_inventory().unwrap().digest;
    println!("inventory_digest_equal={}", digest_before == digest_after);
    // Verdicts are rejudged through the facades by `signature_compaction_check`.
}
