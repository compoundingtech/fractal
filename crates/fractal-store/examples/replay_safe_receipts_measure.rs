//! Measure the replay-safe receipt pass on a copy of a real member's database: receipts and bytes
//! before and after, each writer loan's hold, the free pages given back, and the replication
//! inventory digest before and after (receipts are outside it). The runtime's own rule decides
//! which receipts stay, as in the daemon.
//! `cargo run --release -p fractal-store --example replay_safe_receipts_measure -- --copy COPY ORIGIN [WINDOW_MS]`.
//! Run it on a copy only: it deletes receipts.
use fractal_store::store::Store;
use smallclaims::store::idempotency::REPLAY_SAFE_WINDOW_MS;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Holds(Vec<Duration>);

impl Holds {
    fn time<T>(&mut self, work: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = work();
        self.0.push(started.elapsed());
        result
    }

    fn report(&mut self, name: &str) {
        let slow = self.0.iter().filter(|hold| hold.as_millis() >= 100).count();
        let which = self.0.iter().enumerate().filter(|(_, hold)| hold.as_millis() >= 100)
            .map(|(index, hold)| format!("#{index}:{}ms", hold.as_millis())).collect::<Vec<_>>();
        if !which.is_empty() {
            println!("{name}: slow loans {}", which.join(" "));
        }
        self.0.sort();
        let at = |q: f64| self.0.get(((self.0.len() as f64 - 1.0) * q) as usize).copied().unwrap_or_default();
        println!(
            "{name}: loans={} at_least_100ms={slow} total_ms={} p50_ms={:.2} p99_ms={:.2} max_ms={:.2}",
            self.0.len(),
            self.0.iter().sum::<Duration>().as_millis(),
            at(0.5).as_secs_f64() * 1e3,
            at(0.99).as_secs_f64() * 1e3,
            at(1.0).as_secs_f64() * 1e3,
        );
    }
}

fn pragma(store: &Store, name: &str) -> u64 {
    store.readers.get().query_row(&format!("PRAGMA {name}"), [], |row| row.get(0)).unwrap()
}

/// Receipts, their bytes, the replay-safe share, and the pages of the table and its index.
fn receipts(store: &Store, when: &str) {
    let reader = store.readers.get();
    let (rows, bytes, safe, safe_bytes): (u64, u64, u64, u64) = reader
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(length(response)+length(operation_id)),0),
                    COALESCE(SUM(replay_safe<>0),0),
                    COALESCE(SUM(CASE WHEN replay_safe<>0 THEN length(response)+length(operation_id) END),0)
             FROM idempotency",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    let pages: u64 = reader
        .query_row(
            "SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name IN ('idempotency','sqlite_autoindex_idempotency_1')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    println!("{when}: receipts={rows} bytes={bytes} replay_safe={safe} replay_safe_bytes={safe_bytes} table_and_index_bytes={pages}");
}

fn main() {
    let mut args = std::env::args().skip(1);
    assert_eq!(args.next().as_deref(), Some("--copy"), "run this on a copy: --copy COPY ORIGIN [WINDOW_MS]");
    let path = args.next().expect("COPY");
    let origin = args.next().expect("ORIGIN");
    let window = args.next().map(|value| value.parse().unwrap()).unwrap_or(REPLAY_SAFE_WINDOW_MS);
    let opened = Instant::now();
    let store = Store::open(std::path::Path::new(&path), origin).unwrap();
    println!("open_ms={} window_ms={window}", opened.elapsed().as_millis());
    let before = store.replication_inventory().unwrap().digest;
    println!("inventory_digest_before={before}");
    let page_size = pragma(&store, "page_size");
    println!("file_bytes_before={} free_before={}", pragma(&store, "page_count") * page_size, pragma(&store, "freelist_count"));
    receipts(&store, "before");

    let mut holds = Holds::default();
    let now = smallclaims::store::now_ms() as i64;
    let started = Instant::now();
    let (mut examined, mut deleted, mut passed) = (0, 0, 0);
    loop {
        let cleaned = holds.time(|| store.cleanup_replay_safe_idempotency(now, window).unwrap());
        examined += cleaned.examined;
        deleted += cleaned.deleted;
        passed += cleaned.passed;
        if cleaned.examined + cleaned.passed == 0 {
            break;
        }
        // The daemon rests 20 ms between chunks.
        std::thread::sleep(Duration::from_millis(20));
    }
    println!("replay_safe_examined={examined} replay_safe_deleted={deleted} published_passed={passed} kept_by_runtime={} pass_s={:.1}",
        examined - deleted, started.elapsed().as_secs_f64());
    holds.report("replay_safe_pass");
    receipts(&store, "after");

    // As `release_free_pages` does: 128 pages per write, until none are free.
    let mut holds = Holds::default();
    let free = pragma(&store, "freelist_count");
    let mut released = 0;
    if pragma(&store, "auto_vacuum") == 2 {
        loop {
            let pages = holds.time(|| {
                let connection = store.connection.write_background();
                let before: u64 = connection.query_row("PRAGMA freelist_count", [], |row| row.get(0)).unwrap();
                let mut statement = connection.prepare("PRAGMA incremental_vacuum(128)").unwrap();
                let mut steps = statement.query([]).unwrap();
                while steps.next().unwrap().is_some() {}
                drop(steps);
                drop(statement);
                let after: u64 = connection.query_row("PRAGMA freelist_count", [], |row| row.get(0)).unwrap();
                before - after
            });
            released += pages;
            if pages == 0 {
                break;
            }
        }
    }
    println!("free_after_deletes={free} pages_released={released}");
    holds.report("release_free_pages");
    println!("file_bytes_after={} free_after={}", pragma(&store, "page_count") * page_size, pragma(&store, "freelist_count"));
    let after = store.replication_inventory().unwrap().digest;
    println!("inventory_digest_after={after} equal={}", before == after);
}
