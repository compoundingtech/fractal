//! Run in release mode on a private copy of a store, never a live one: it empties the copy's
//! valid records' bytes the way the daemon does after an upgrade (`record_bytes`), then gives
//! the freed pages back. Prints only timings, page counts and digests, not stored content.
use fractal_daemon::store::Store;
use std::{path::Path, sync::Arc, time::{Duration, Instant}};

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let path = Path::new(&args[1]);
    let fleet = args.get(2).cloned();
    let pages = |label: &str| -> anyhow::Result<()> {
        let connection = rusqlite::Connection::open(path)?;
        let (count, free): (u64, u64) = connection.query_row(
            "SELECT (SELECT page_count FROM pragma_page_count), (SELECT freelist_count FROM pragma_freelist_count)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        println!("{label}_pages={count} {label}_free_pages={free} {label}_file_bytes={}", std::fs::metadata(path)?.len());
        Ok(())
    };
    pages("before")?;
    let origin: String = rusqlite::Connection::open(path)?.query_row(
        "SELECT origin FROM batches ORDER BY rowid DESC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let started = Instant::now();
    let store = Arc::new(Store::open(path, &origin)?);
    println!("open_ms={}", started.elapsed().as_millis());
    // What opening the copy with this build migrated, apart from what the pass frees.
    pages("opened")?;
    // The daemon's WAL checkpointer, every five seconds, as `recycle_idle_wal` runs it.
    {
        let store = Arc::downgrade(&store);
        let path = path.to_owned();
        std::thread::spawn(move || {
            let connection = rusqlite::Connection::open(&path).unwrap();
            let mut growth = smallclaims::store::wal_escalation::WalGrowth::default();
            let origin = Instant::now();
            while let Some(store) = store.upgrade() {
                let started = Instant::now();
                let _ = store.recycle_wal(&connection, &mut growth);
                let took = started.elapsed();
                if took.as_millis() >= 20 {
                    println!("checkpoint_at_ms={} checkpoint_ms={}", (started - origin).as_millis(), took.as_millis());
                }
                drop(store);
                std::thread::sleep(Duration::from_secs(5));
            }
        });
    }
    let digests = |store: &Store| -> anyhow::Result<(String, String)> {
        let status = store.replication_status(true, fleet.as_deref(), &[])?;
        Ok((status.authority_digest, status.graph_digest))
    };
    let before = digests(&store)?;
    let mut holds = Vec::new();
    let (mut emptied, mut bytes) = (0, 0);
    let started = Instant::now();
    loop {
        let page_started = Instant::now();
        let page = store.release_valid_record_bytes()?;
        let took = page_started.elapsed();
        if took.as_millis() >= 100 {
            println!("slow_empty_ms={} work_ms={} rows={}", took.as_millis(), page.work.as_millis(), page.emptied);
        }
        holds.push(took);
        emptied += page.emptied;
        bytes += page.bytes;
        if page.done {
            break;
        }
    }
    println!("emptied={emptied} emptied_bytes={bytes} empty_ms={}", started.elapsed().as_millis());
    report("empty", &mut holds);
    let mut released = 0;
    let started = Instant::now();
    loop {
        let step_started = Instant::now();
        let step = store.release_free_pages_in(smallclaims::store::record_bytes::FREE_PAGES_PER_WRITE)?;
        let took = step_started.elapsed();
        if took.as_millis() >= 100 {
            println!("slow_release_ms={}", took.as_millis());
        }
        holds.push(took);
        if step == 0 {
            break;
        }
        released += step;
    }
    println!("released_pages={released} release_ms={}", started.elapsed().as_millis());
    report("release", &mut holds);
    let after = digests(&store)?;
    println!("authority_equal={} graph_equal={}", before.0 == after.0, before.1 == after.1);
    println!("authority_digest={} graph_digest={}", after.0, after.1);
    drop(store);
    std::thread::sleep(Duration::from_secs(6));
    pages("after")?;
    Ok(())
}

/// Each writer job's time from submission to commit, the upper bound of its writer hold.
fn report(label: &str, holds: &mut Vec<std::time::Duration>) {
    holds.sort();
    let at = |fraction: f64| holds[((holds.len() - 1) as f64 * fraction) as usize].as_secs_f64() * 1000.0;
    if !holds.is_empty() {
        println!(
            "{label}_jobs={} {label}_hold_p50_ms={:.1} {label}_hold_p99_ms={:.1} {label}_hold_max_ms={:.1}",
            holds.len(), at(0.5), at(0.99), at(1.0)
        );
    }
    holds.clear();
}
