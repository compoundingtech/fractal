//! Measure dropping the verdict links' index by claim on a copy of a real member's database: the
//! drop itself (as this release's open runs it), the pages it gives back and each release's hold,
//! one full pass of the orphan sweep with each loan's writer hold, the rebuild a rollback to the
//! previous release runs at its open, and the replication inventory digest before and after.
//! `cargo run --release -p smallclaims --example verdict_link_index_measure -- --copy COPY ORIGIN`.
//! Run it on a copy only: it writes.
use smallclaims::Store;
use smallclaims::store::runtime::Plain;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Holds(Vec<Duration>);

impl Holds {
    fn report(&mut self, name: &str) {
        let slow = self.0.iter().enumerate().filter(|(_, hold)| hold.as_millis() >= 100)
            .map(|(index, hold)| format!("#{index}:{}ms", hold.as_millis())).collect::<Vec<_>>();
        if !slow.is_empty() {
            println!("{name}: slow loans {}", slow.join(" "));
        }
        let total: Duration = self.0.iter().sum();
        self.0.sort();
        let at = |q: f64| self.0.get(((self.0.len() as f64 - 1.0) * q) as usize).copied().unwrap_or_default();
        println!(
            "{name}: loans={} at_least_100ms={} total_s={:.1} p50_ms={:.2} p99_ms={:.2} max_ms={:.2}",
            self.0.len(), slow.len(), total.as_secs_f64(),
            at(0.5).as_secs_f64() * 1e3, at(0.99).as_secs_f64() * 1e3, at(1.0).as_secs_f64() * 1e3,
        );
    }
}

const INDEX: &str = "claim_candidate_verdict_links_claim_index";

fn pragma(connection: &rusqlite::Connection, name: &str) -> u64 {
    connection.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0)).unwrap()
}

fn index_bytes(connection: &rusqlite::Connection) -> u64 {
    connection.query_row("SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name=?1", [INDEX], |row| row.get(0)).unwrap()
}

fn main() {
    let mut args = std::env::args().skip(1);
    assert_eq!(args.next().as_deref(), Some("--copy"), "run this on a copy: --copy COPY ORIGIN");
    let path = args.next().expect("COPY");
    let origin = args.next().expect("ORIGIN");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.busy_timeout(Duration::from_secs(30)).unwrap();
    let page_size = pragma(&raw, "page_size");
    let links: u64 = raw.query_row("SELECT COUNT(*) FROM claim_verdict_links", [], |row| row.get(0)).unwrap();
    println!("file_bytes_before={} links={links} index_bytes={}", pragma(&raw, "page_count") * page_size, index_bytes(&raw));

    // As this release's open runs it, in one transaction.
    let started = Instant::now();
    raw.execute_batch(&format!("BEGIN IMMEDIATE; DROP INDEX IF EXISTS {INDEX}; COMMIT;")).unwrap();
    println!("drop_index_ms={} free_after_drop={}", started.elapsed().as_millis(), pragma(&raw, "freelist_count"));
    drop(raw);

    let opened = Instant::now();
    let store = Store::open(std::path::Path::new(&path), &origin, Arc::new(Plain)).unwrap();
    println!("open_ms={}", opened.elapsed().as_millis());
    let before = store.replication_inventory().unwrap().digest;

    // The freed pages go back 128 per write, as `release_free_pages` does.
    let mut holds = Holds::default();
    loop {
        let started = Instant::now();
        let released = {
            let connection = store.connection.write_background();
            let before: u64 = connection.query_row("PRAGMA freelist_count", [], |row| row.get(0)).unwrap();
            let mut statement = connection.prepare("PRAGMA incremental_vacuum(128)").unwrap();
            let mut steps = statement.query([]).unwrap();
            while steps.next().unwrap().is_some() {}
            drop(steps);
            drop(statement);
            before - connection.query_row::<u64, _, _>("PRAGMA freelist_count", [], |row| row.get(0)).unwrap()
        };
        holds.0.push(started.elapsed());
        if released == 0 {
            break;
        }
    }
    holds.report("release_free_pages");
    let file_after: u64 = store.readers.get().query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0)).unwrap() * page_size;
    println!("file_bytes_after_release={file_after}");

    // One full pass of the sweep, paced as the daemon paces it.
    let mut holds = Holds::default();
    let started = Instant::now();
    let now = smallclaims::store::now_ms() as u64;
    let (mut examined, mut deleted) = (0, 0);
    loop {
        let page_started = Instant::now();
        let page = store.sweep_orphan_verdict_links(now).unwrap();
        holds.0.push(page.held);
        examined += page.examined;
        deleted += page.deleted;
        if page.finished {
            break;
        }
        std::thread::sleep(page_started.elapsed());
    }
    println!("sweep: examined={examined} orphans_deleted={deleted} pass_wall_s={:.0}", started.elapsed().as_secs_f64());
    holds.report("sweep_writer_hold");
    let after = store.replication_inventory().unwrap().digest;
    println!("inventory_digest_equal={}", before == after);
    drop(store);

    // What the previous release runs at its open after a rollback, in one transaction.
    let raw = rusqlite::Connection::open(&path).unwrap();
    let started = Instant::now();
    raw.execute_batch(&format!("BEGIN IMMEDIATE; CREATE INDEX IF NOT EXISTS {INDEX} ON claim_verdict_links(claim_id); COMMIT;")).unwrap();
    println!("rollback_rebuild_ms={} rebuilt_index_bytes={}", started.elapsed().as_millis(), index_bytes(&raw));
}
