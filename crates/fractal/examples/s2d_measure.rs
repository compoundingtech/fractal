//! Run in release mode on a private copy of a store, never a live one. It runs the passes this
//! build runs after an upgrade, with their switches forced on for the copy only: emptying valid
//! records' bytes (S1), deleting valid records with no rollback window (S2b), and releasing
//! envelopes (S2d) with the age gate measured from the copy's newest envelope rather than from
//! now, so the copy's age profile is the member's, and deleting what peers retired (a retirement
//! wins). Then it gives the freed pages back. It prints
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
    // S2D_PASSES picks the passes (sign, empty, delete, release, retire, vacuum; all but retire
    // by default) and S2D_NO_SPACE skips the byte counts, to measure one pass again on a copy
    // already measured. S2D_RETIRE_CLAIMS sets how many retirements retire seeds (a day's cap).
    let passes_env = std::env::var("S2D_PASSES").unwrap_or_else(|_| "sign,empty,delete,release,vacuum".into());
    let passes: Vec<&str> = passes_env.split(',').collect();
    let count_space = std::env::var_os("S2D_NO_SPACE").is_none();
    let before_space = if count_space { space(path, "before")? } else { BTreeMap::new() };
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
    // A copy has no peers, so nothing teaches it a retirement: retire seeds what a peer whose cut
    // is later would retire, a day's cap of the observed claims a newer one of the same subject
    // and kind replaced, as the claim-range walk records them. Seeded before the control starts.
    let retire_claims: u64 = std::env::var("S2D_RETIRE_CLAIMS").ok().and_then(|claims| claims.parse().ok())
        .unwrap_or(smallclaims::store::checkpoint_trim::LOCAL_TRIM_DAILY_CLAIMS);
    if passes.contains(&"retire") {
        let started = Instant::now();
        let mut seeded = Vec::new();
        {
            let connection = store.readers.get();
            let mut replaced = connection.prepare(
                "SELECT id, kind, actor FROM claims AS held WHERE kind=?1 AND EXISTS(SELECT 1 FROM claims AS newer
                   WHERE newer.subject=held.subject AND newer.kind=held.kind
                     AND CAST(newer.accepted_at_unix_ms AS INTEGER) > CAST(held.accepted_at_unix_ms AS INTEGER))
                 ORDER BY CAST(accepted_at_unix_ms AS INTEGER) LIMIT ?2",
            )?;
            for kind in smallclaims::store::checkpoint_trim::LOCAL_TRIM_KINDS {
                let left = retire_claims.saturating_sub(seeded.len() as u64);
                let rows = replaced
                    .query_map(rusqlite::params![kind, left], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                seeded.extend(rows);
            }
        }
        let pending = seeded.iter().filter(|(_, kind, actor)| smallclaims::store::retirement_wins::deletable_on_a_peers_word(kind, actor.as_deref())).count();
        let now = i64::try_from(smallclaims::store::now_ms())?;
        store.connection.batched(|tx| -> rusqlite::Result<()> {
            let mut insert = tx.prepare("INSERT OR IGNORE INTO peer_retired_claims(id, peer, state, learned_at_unix_ms) VALUES (?1, 'peer/measure', ?2, ?3)")?;
            for (id, kind, actor) in &seeded {
                let state = if smallclaims::store::retirement_wins::deletable_on_a_peers_word(kind, actor.as_deref()) { "pending" } else { "kept" };
                insert.execute(rusqlite::params![id, state, now])?;
            }
            Ok(())
        }).map_err(|error| anyhow::anyhow!(error))??;
        println!("retire_seeded={} retire_pending={pending} seed_ms={}", seeded.len(), started.elapsed().as_millis());
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
                if job.elapsed() >= Duration::from_millis(100) {
                    println!("slow_job=control at_unix_ms={} hold_ms={}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(), job.elapsed().as_millis());
                }
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

    if passes.contains(&"sign") {
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
    }

    if passes.contains(&"empty") {
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
    }

    if passes.contains(&"delete") {
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
    }

    if passes.contains(&"release") {
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
        if std::env::var_os("S2D_RESET_RELEASE").is_some() {
            // Judge every envelope again, as a first pass would.
            store.connection.batched(|tx| tx.execute("DELETE FROM meta WHERE key='envelope_release_cursor'", []))
                .map_err(|error| anyhow::anyhow!(error))??;
        }
        let (mut released, mut read, mut unsigned_bytes) = (0, 0, 0);
        let mut kept = BTreeMap::<&str, usize>::new();
        let started = Instant::now();
        loop {
            let job = Instant::now();
            let page = store.release_envelopes(min_age)?;
            // A page judges on a reader first; the writer is held from its submission on.
            let took = job.elapsed().saturating_sub(page.judged);
            slow("release_envelopes", took, format!("work_ms={} judged_ms={} read={} released={}", page.work.as_millis(), page.judged.as_millis(), page.scanned, page.released));
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
    }

    if passes.contains(&"retire") {
        // A retirement wins: the checkpoint worker's retirement step deletes the pending ones through
        // the trim's paced chunks, each its own write. A step is many writes, so the writer holds
        // are read from the control writer: its waits while the steps run, beside a quiet minute.
        store.trim_locally(true)?;
        store.trim_locally_at_most(retire_claims);
        // A cut the member's own trim left `trimming` holds every retirement back, as it would on
        // the member. S2D_RETIRE_SET_ASIDE sets it aside on the copy, so the steps measure the
        // retirements' own deletion rather than the cut's backlog.
        if std::env::var_os("S2D_RETIRE_SET_ASIDE").is_some() {
            let set_aside = store.connection.batched(|tx| tx.execute("UPDATE checkpoints SET state='set-aside' WHERE state='trimming'", []))
                .map_err(|error| anyhow::anyhow!(error))??;
            println!("retire_cuts_set_aside={set_aside}");
        }
        let pending = |store: &Store| -> anyhow::Result<u64> {
            Ok(store.readers.get().query_row("SELECT COUNT(*) FROM peer_retired_claims WHERE state='pending'", [], |row| row.get(0))?)
        };
        // What the step's last write costs at this many pending: what is gone no longer waits.
        let probe = Instant::now();
        store.connection.batched(|tx| tx.execute(
            "DELETE FROM peer_retired_claims WHERE state='pending' AND NOT EXISTS(SELECT 1 FROM claims WHERE claims.id=peer_retired_claims.id)",
            [],
        )).map_err(|error| anyhow::anyhow!(error))??;
        println!("retire_cleanup_probe_ms={} pending={}", probe.elapsed().as_millis(), pending(&store)?);
        let quiet = control.lock().unwrap().len();
        std::thread::sleep(Duration::from_secs(60));
        let from = control.lock().unwrap().len();
        let mut quiet_holds = control.lock().unwrap()[quiet..from].to_vec();
        let started = Instant::now();
        let mut deleted = 0;
        loop {
            let waiting = pending(&store)?;
            let job = Instant::now();
            // The retirements' step alone: the member's own cut is the trim's business.
            let actions = store.trim_peer_retired_step()?;
            let took = job.elapsed();
            let mut stop = actions.is_empty();
            for action in &actions {
                println!("retire_step_ms={} action={}", took.as_millis(), serde_json::to_string(action)?);
                match action {
                    smallclaims::store::checkpoint_agreement::CheckpointAction::Trimmed { claims, .. } => deleted += claims,
                    smallclaims::store::checkpoint_agreement::CheckpointAction::TrimGraphChanged { .. } => stop = true,
                    _ => {}
                }
            }
            if stop || pending(&store)? == 0 || pending(&store)? == waiting {
                break;
            }
        }
        let to = control.lock().unwrap().len();
        let mut retire_holds = control.lock().unwrap()[from..to].to_vec();
        println!(
            "retire_deleted={deleted} retire_left_pending={} retire_kept={} retire_ms={}",
            pending(&store)?,
            store.peer_retired_claims_kept()?,
            started.elapsed().as_millis()
        );
        println!("control_quiet_over_100ms={}", quiet_holds.iter().filter(|took| took.as_millis() >= 100).count());
        report("control_quiet", &mut quiet_holds);
        println!("control_retire_over_100ms={}", retire_holds.iter().filter(|took| took.as_millis() >= 100).count());
        report("control_retire", &mut retire_holds);
    }

    if passes.contains(&"vacuum") {
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
    }

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
    let after_space = if count_space { space(path, "after")? } else { BTreeMap::new() };
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
