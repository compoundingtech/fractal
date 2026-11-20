//! Measure the settled-judging prune, response expiry and free-page release on a copy of a real
//! member's database: rows deleted, each writer loan's hold, pages given back, and the
//! replication inventory digest before and after (the pruned tables are outside it).
//! Usage: settled_judging_measure COPY ORIGIN. Run it on a copy only: it writes.
use smallclaims::Store;
use smallclaims::store::runtime::Plain;
use std::sync::Arc;
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
        let slow = self
            .0
            .iter()
            .enumerate()
            .filter(|(_, hold)| hold.as_millis() >= 100)
            .map(|(index, hold)| format!("#{index}:{}ms", hold.as_millis()))
            .collect::<Vec<_>>();
        println!("{name}: loans_at_least_100ms={} [{}]", slow.len(), slow.iter().take(20).cloned().collect::<Vec<_>>().join(" "));
        self.0.sort();
        let total: Duration = self.0.iter().sum();
        let at = |q: f64| self.0.get(((self.0.len() as f64 - 1.0) * q) as usize).copied().unwrap_or_default();
        println!(
            "{name}: loans={} total_ms={} p50_ms={:.2} p99_ms={:.2} max_ms={:.2}",
            self.0.len(),
            total.as_millis(),
            at(0.5).as_secs_f64() * 1e3,
            at(0.99).as_secs_f64() * 1e3,
            at(1.0).as_secs_f64() * 1e3,
        );
    }
}

fn pragma(store: &Store, name: &str) -> u64 {
    store.readers.get().query_row(&format!("PRAGMA {name}"), [], |row| row.get(0)).unwrap()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("COPY");
    let origin = args.next().expect("ORIGIN");
    let opened = Instant::now();
    let store = Store::open(std::path::Path::new(&path), &origin, Arc::new(Plain)).unwrap();
    println!("open_ms={}", opened.elapsed().as_millis());
    println!("migration_done={}", store.signature_migration().unwrap().done());
    let before = store.replication_inventory().unwrap().digest;
    println!("inventory_digest_before={before}");
    println!("pages_before={} free_before={}", pragma(&store, "page_count"), pragma(&store, "freelist_count"));

    let mut holds = Holds::default();
    let (mut examined, mut deleted) = (0, 0);
    loop {
        let page = holds.time(|| store.prune_settled_signature_judgments().unwrap());
        examined += page.examined;
        deleted += page.deleted;
        if page.finished || page.examined == 0 {
            break;
        }
    }
    println!("judging_examined={examined} judging_deleted={deleted}");
    holds.report("judging_prune");

    let mut holds = Holds::default();
    let now = smallclaims::store::now_ms() as i64;
    let (mut expired, mut extended) = (0, 0);
    loop {
        let cleaned = holds.time(|| store.cleanup_idempotency(now).unwrap());
        expired += cleaned.deleted;
        extended += cleaned.extended;
        if cleaned.examined == 0 {
            break;
        }
    }
    println!("idempotency_deleted={expired} idempotency_extended={extended}");
    holds.report("response_expiry");

    // As `release_free_pages` does: FREE_PAGE_CHUNK (128) pages per write, until none are free.
    let mut holds = Holds::default();
    let free = pragma(&store, "freelist_count");
    let mut released = 0;
    let chunk: u64 = std::env::var("FREE_PAGE_CHUNK").ok().and_then(|value| value.parse().ok()).unwrap_or(128);
    println!("free_page_chunk={chunk}");
    if pragma(&store, "auto_vacuum") == 2 {
        loop {
            let pages = holds.time(|| {
                let connection = store.connection.write_background();
                let before: u64 = connection.query_row("PRAGMA freelist_count", [], |row| row.get(0)).unwrap();
                let mut statement = connection.prepare(&format!("PRAGMA incremental_vacuum({chunk})")).unwrap();
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
    println!("pages_after={} free_after={}", pragma(&store, "page_count"), pragma(&store, "freelist_count"));
    let after = store.replication_inventory().unwrap().digest;
    println!("inventory_digest_after={after} equal={}", before == after);
    let verdicts: u64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM claim_verdicts WHERE verdict='verified'", [], |row| row.get(0))
        .unwrap();
    println!("verified_after={verdicts} recheck_mismatches=skipped");
}
