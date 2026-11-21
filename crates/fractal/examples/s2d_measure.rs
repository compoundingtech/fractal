//! Run in release mode on a private copy of a store, never a live one. It runs the passes this
//! build runs after an upgrade, with their switches forced on for the copy only: emptying valid
//! records' bytes (S1), deleting valid records with no rollback window (S2b), and releasing
//! envelopes (S2d) with the age gate measured from the copy's newest envelope rather than from
//! now, so the copy's age profile is the member's. Then it gives the freed pages back. It prints
//! bytes per table, file size, each job's writer hold, the digests before and after, and why
//! envelopes stay. Only counts, timings and digests are printed, never stored content.
//!
//! With STATE_DIR, the copy is prepared with that member's fleet file and key as the daemon
//! prepares its store, and the member's own claims are signed first (S2c) as its daemon would.
//! The key is read, never printed. A control writer commits one row every 50 ms throughout; its
//! holds are reported beside the passes'.
//!
//! ```sh
//! cargo run --release -p fractal --example s2d_measure -- COPY.sqlite3 NODE [STATE_DIR]
//! ```
use fractal_daemon::store::Store;
use std::{collections::BTreeMap, path::Path, sync::Arc, time::{Duration, Instant}};

/// Bytes each table holds with its indexes (`dbstat`), and the file's pages.
fn space(path: &Path, label: &str) -> anyhow::Result<BTreeMap<String, u64>> {
    let connection = rusqlite::Connection::open(path)?;
    let (count, free, size): (u64, u64, u64) = connection.query_row(
        "SELECT (SELECT page_count FROM pragma_page_count), (SELECT freelist_count FROM pragma_freelist_count),
                (SELECT page_size FROM pragma_page_size)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    println!(
        "{label}_pages={count} {label}_free_pages={free} {label}_page_size={size} {label}_file_bytes={}",
        std::fs::metadata(path)?.len()
    );
    let started = Instant::now();
    let tables = connection
        .prepare(
            "SELECT COALESCE(master.tbl_name, stat.name), SUM(stat.pgsize) FROM dbstat AS stat
             LEFT JOIN sqlite_master AS master ON master.name=stat.name
             GROUP BY 1",
        )?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)))?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    println!("{label}_dbstat_ms={}", started.elapsed().as_millis());
    Ok(tables)
}

/// Every digest a peer compares: the claim range digests of both namespaces, each projection
/// digest, and the graph and authority digests.
fn digests(store: &Store, fleet: Option<&str>) -> anyhow::Result<BTreeMap<String, String>> {
    store.fold_claim_ranges_paced(|_| {})?;
    let mut all = BTreeMap::new();
    let ranges = store.claim_range_summary()?;
    all.insert("claim_ranges.retained".into(), format!("{:?}", ranges.retained));
    all.insert("claim_ranges.retired".into(), format!("{:?}", ranges.retired.as_ref().map(|retired| &retired.retained)));
    let status = store.replication_status(true, fleet, &[])?;
    all.insert("authority".into(), status.authority_digest);
    all.insert("graph".into(), status.graph_digest);
    for (table, digest) in status.projection_digests {
        all.insert(format!("projection.{table}"), digest);
    }
    Ok(all)
}

/// Each writer job's time from submission to commit, the upper bound of its writer hold.
fn report(label: &str, holds: &mut Vec<Duration>) {
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

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let path = Path::new(&args[1]);
    // The member the copy belongs to: its own claims are the ones it signs and serves.
    let node = args.get(2).ok_or_else(|| anyhow::anyhow!("the copy's node name is required"))?;
    let state_dir = args.get(3).map(Path::new);
    let run = Instant::now();
    let before_space = space(path, "before")?;
    let started = Instant::now();
    let store = Arc::new(Store::open(path, node)?);
    println!("open_ms={}", started.elapsed().as_millis());
    let fleet = match state_dir {
        Some(state_dir) => {
            let file = smallclaims::fleet::file::FleetFile::load(state_dir)?
                .ok_or_else(|| anyhow::anyhow!("no fleet file in the state directory"))?;
            smallclaims::fleet::prepare(&store, state_dir, &file)?;
            Some(file.fleet_id)
        }
        None => None,
    };
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
    let before = digests(&store, fleet.as_deref())?;
    // The control: one short write every 50 ms, as a busy member's own appends would queue.
    let control = Arc::new(std::sync::Mutex::new(Vec::<Duration>::new()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let control_thread = {
        let (store, control, stop) = (store.clone(), control.clone(), stop.clone());
        store.connection.batched(|tx| tx.execute_batch("CREATE TABLE IF NOT EXISTS measure_control(at INTEGER)"))
            .map_err(|error| anyhow::anyhow!(error))??;
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let job = Instant::now();
                let _ = store.connection.batched(|tx| tx.execute("INSERT INTO measure_control VALUES (1)", []));
                control.lock().unwrap().push(job.elapsed());
                std::thread::sleep(Duration::from_millis(50));
            }
        })
    };
    let clock = Instant::now();
    let slow = |job: &str, took: Duration, detail: String| {
        if took.as_millis() >= 100 {
            println!("slow_job={job} at_ms={} hold_ms={} {detail}", clock.elapsed().as_millis(), took.as_millis());
        }
    };
    let mut holds = Vec::new();

    // S2c: this member signs its own claims, settling waiting signatures first, a page at a time.
    let (mut scanned, mut signed) = (0, 0);
    let started = Instant::now();
    loop {
        let job = Instant::now();
        store.promote_claim_member_signatures()?;
        let page = store.sign_own_claims()?;
        let took = job.elapsed();
        slow("sign_own", took, format!("scanned={} signed={}", page.scanned, page.signed));
        holds.push(took);
        scanned += page.scanned;
        signed += page.signed;
        if page.done {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("own_claims_scanned={scanned} own_claims_signed={signed} sign_ms={}", started.elapsed().as_millis());
    report("sign", &mut holds);

    // S1: valid records keep no bytes.
    let (mut emptied, mut bytes) = (0, 0);
    let started = Instant::now();
    loop {
        let job = Instant::now();
        let page = store.release_valid_record_bytes()?;
        let took = job.elapsed();
        slow("empty_bytes", took, format!("work_ms={} rows={}", page.work.as_millis(), page.emptied));
        holds.push(took);
        emptied += page.emptied;
        bytes += page.bytes;
        if page.done {
            break;
        }
    }
    println!("emptied={emptied} emptied_bytes={bytes} empty_ms={}", started.elapsed().as_millis());
    report("empty", &mut holds);

    // S2b: valid records deleted, with no rollback window.
    let (mut scanned, mut deleted) = (0, 0);
    let started = Instant::now();
    loop {
        let job = Instant::now();
        let page = store.release_valid_records(0)?;
        let took = job.elapsed();
        slow("delete_records", took, format!("work_ms={} rows={}", page.work.as_millis(), page.deleted));
        holds.push(took);
        scanned += page.scanned;
        deleted += page.deleted;
        if page.done || page.waiting_until_unix_ms.is_some() {
            break;
        }
    }
    println!("records_scanned={scanned} records_deleted={deleted} delete_ms={}", started.elapsed().as_millis());
    report("delete", &mut holds);

    // S2d: envelopes released once 14 days older than the copy's newest envelope.
    let newest: u64 = store.readers.get().query_row(
        "SELECT MAX(CAST(accepted_at_unix_ms AS INTEGER)) FROM replica_envelopes",
        [],
        |row| row.get(0),
    )?;
    let gate = newest.saturating_sub(smallclaims::store::envelope_release::ENVELOPE_RELEASE_AGE_MS);
    let min_age = u64::try_from(smallclaims::store::now_ms())?.saturating_sub(gate);
    let (envelopes, young): (u64, u64) = store.readers.get().query_row(
        "SELECT COUNT(*), SUM(CAST(accepted_at_unix_ms AS INTEGER) > ?1) FROM replica_envelopes",
        [gate],
        |row| Ok((row.get(0)?, row.get::<_, Option<u64>>(1)?.unwrap_or(0))),
    )?;
    println!("envelopes={envelopes} younger_than_gate={young} newest_accepted_ms={newest}");
    let (mut released, mut read, mut unsigned_bytes) = (0, 0, 0);
    let mut kept = BTreeMap::<&str, usize>::new();
    let started = Instant::now();
    loop {
        let job = Instant::now();
        let page = store.release_envelopes(min_age)?;
        let took = job.elapsed();
        slow("release_envelopes", took, format!("work_ms={} read={} released={}", page.work.as_millis(), page.scanned, page.released));
        holds.push(took);
        if let Some(waiting) = page.waiting {
            println!("release_waiting={waiting:?}");
        }
        released += page.released;
        read += page.scanned;
        unsigned_bytes += page.unsigned_payload_bytes;
        for (reason, count) in page.kept {
            *kept.entry(reason).or_default() += count;
        }
        if page.done {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!(
        "envelopes_read={read} envelopes_released={released} release_ms={} unsigned_payload_bytes={unsigned_bytes}",
        started.elapsed().as_millis()
    );
    for (reason, count) in &kept {
        println!("kept_reason={reason} kept={count}");
    }
    report("release", &mut holds);

    // The pages they held, back to the filesystem.
    let mut pages = 0;
    let started = Instant::now();
    loop {
        let job = Instant::now();
        let step = store.release_free_pages_in(smallclaims::store::record_bytes::FREE_PAGES_PER_WRITE)?;
        let took = job.elapsed();
        slow("free_pages", took, format!("pages={step}"));
        holds.push(took);
        if step == 0 {
            break;
        }
        pages += step;
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("vacuum_pages={pages} vacuum_ms={}", started.elapsed().as_millis());
    report("vacuum", &mut holds);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    control_thread.join().map_err(|_| anyhow::anyhow!("the control writer panicked"))?;
    let mut control = std::mem::take(&mut *control.lock().unwrap());
    println!("control_over_100ms={}", control.iter().filter(|took| took.as_millis() >= 100).count());
    report("control", &mut control);

    let after = digests(&store, fleet.as_deref())?;
    let differing = before.iter().filter(|(name, digest)| after.get(*name) != Some(digest)).map(|(name, _)| name.as_str()).collect::<Vec<_>>();
    println!("digests_compared={} digests_equal={} differing={differing:?}", before.len(), differing.is_empty());
    drop(store);
    std::thread::sleep(Duration::from_secs(6));
    let after_space = space(path, "after")?;
    let mut freed = before_space
        .iter()
        .map(|(table, bytes)| (table.clone(), *bytes as i64 - *after_space.get(table).unwrap_or(&0) as i64))
        .collect::<Vec<_>>();
    freed.sort_by_key(|(_, bytes)| -bytes.abs());
    for (table, bytes) in freed.iter().take(15) {
        println!("freed_table={table} freed_bytes={bytes}");
    }
    println!(
        "freed_total_bytes={} run_ms={}",
        before_space.values().sum::<u64>() as i64 - after_space.values().sum::<u64>() as i64,
        run.elapsed().as_millis()
    );
    Ok(())
}
