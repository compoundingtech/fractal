use super::*;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// The frames in a store's WAL, from the WAL-index header's `mxFrame`, so looking checkpoints
/// nothing.
fn wal_frames(path: &std::path::Path) -> i32 {
    let mut shm = path.as_os_str().to_owned();
    shm.push("-shm");
    let header = std::fs::read(shm).unwrap_or_default();
    header.get(16..20).map_or(0, |frames| i32::from_ne_bytes(frames.try_into().unwrap()))
}

/// A store an older build wrote: its envelopes carry the old payloads, which the daemon's startup
/// converts, one page a commit with a short rest, as soon as it starts.
fn older_build_store(path: &std::path::Path) {
    let store = Store::open(path, "alder").unwrap();
    store.bind_fleet("sample-fleet").unwrap();
    // Only a member with a key keeps signed envelopes to convert.
    store.set_member_key(Some(Arc::new(fractal_daemon::fleet::MemberKey::generate().unwrap().0))).unwrap();
    for n in 0..1_500 {
        store
            .append_as_older_build_for_test(&ClaimInput {
                subject: "daemon/alder".into(),
                kind: "daemon.diagnostic".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("severity".into(), json!("warning")),
                    ("code".into(), json!("startup-wal")),
                    ("reason".into(), json!(format!("{n} {}", "x".repeat(4_000)))),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some(format!("startup-wal-{n}")),
            })
            .unwrap();
    }
    // Signed into envelopes as an exchange carries them.
    store.export_replication_exchange("sample-fleet", &fractal_daemon::model::ReplicationInventory::default()).unwrap();
    // As an older build stored them: payloads as base64 text, at schema 15, so opening the store
    // converts them again.
    {
        use base64::Engine as _;
        let connection = store.connection.write();
        let payloads: Vec<(i64, Vec<u8>)> = connection
            .prepare("SELECT rowid,payload FROM replica_envelopes")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for (rowid, bytes) in payloads {
            connection
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h4cb8851484ee owner=fractal::wal_escalation_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET payload=?1 WHERE rowid=?2",
                    rusqlite::params![base64::engine::general_purpose::STANDARD.encode(bytes), rowid],
                )
                .unwrap();
        }
        connection.execute_batch("PRAGMA user_version=15").unwrap();
    }
    let converting: i64 = store.readers.get().query_row("SELECT COUNT(*) FROM replica_envelopes WHERE typeof(payload)='text'", [], |row| row.get(0)).unwrap();
    assert_eq!(converting, 1_500, "every envelope waits to be converted");
    assert!(store.checkpoint_idle_wal(&rusqlite::Connection::open(path).unwrap()).unwrap(), "the seed leaves an empty WAL");
}

/// Start a daemon on `path` as `up` does its WAL recycle and its startup payload conversion, with
/// two readers always open as the roster's and work list's first folds keep them, until the
/// conversion is done. The largest WAL it saw.
async fn start(path: &std::path::Path, limits: smallclaims::store::wal_escalation::WalEscalation) -> i32 {
    let store = Arc::new(Store::open(path, "alder").unwrap());
    store.set_wal_escalation(limits);
    let stop = Arc::new(AtomicBool::new(false));
    let largest = Arc::new(AtomicI32::new(0));
    let mut threads = Vec::new();
    for offset in [0, 50] {
        let (store, stop) = (Arc::clone(&store), Arc::clone(&stop));
        threads.push(std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(offset));
            let reader = store.readers.get();
            while !stop.load(Ordering::Acquire) {
                let read = reader.unchecked_transaction().unwrap();
                read.query_row("SELECT COUNT(*) FROM claims", [], |row| row.get::<_, i64>(0)).unwrap();
                std::thread::sleep(Duration::from_millis(100));
                read.commit().unwrap();
            }
        }));
    }
    let (sampled, sampling, watched) = (Arc::clone(&largest), Arc::clone(&stop), path.to_owned());
    threads.push(std::thread::spawn(move || {
        while !sampling.load(Ordering::Acquire) {
            sampled.fetch_max(wal_frames(&watched), Ordering::AcqRel);
            std::thread::sleep(Duration::from_millis(5));
        }
    }));
    // As `up` starts them: the recycle, then the startup maintenance.
    recycle_idle_wal(store.path.clone(), Arc::downgrade(&store), Duration::from_millis(20));
    convert_envelope_payloads(Arc::clone(&store)).await;
    stop.store(true, Ordering::Release);
    threads.into_iter().for_each(|thread| thread.join().unwrap());
    largest.load(Ordering::Acquire)
}

/// A daemon's startup maintenance writes nearly all the time while the first folds keep readers on
/// older snapshots, so the recycle never finds the WAL copied and, without an escalation, the WAL
/// grows until the maintenance ends: 661 MB within two minutes of starting on a 57 GB store. Past
/// its bound and still growing, the recycle restarts it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_starting_daemon_s_wal_recycle_restarts_a_wal_its_startup_work_keeps_growing() {
    let scratch = tempfile::tempdir().unwrap();
    let seeded = scratch.path().join("seeded.sqlite3");
    older_build_store(&seeded);
    let bound = 300;
    let limits = smallclaims::store::wal_escalation::WalEscalation {
        frames: bound,
        samples: 2,
        ..smallclaims::store::wal_escalation::WAL_ESCALATION
    };
    let run = |name: &str| {
        let path = scratch.path().join(name);
        std::fs::copy(&seeded, &path).unwrap();
        path
    };
    let without = start(&run("without.sqlite3"), smallclaims::store::wal_escalation::WalEscalation { frames: i32::MAX, ..limits }).await;
    let escalations = smallclaims::windows::wal::escalations()["count"].as_u64().unwrap();
    let with = start(&run("with.sqlite3"), limits).await;
    let escalated = smallclaims::windows::wal::escalations()["count"].as_u64().unwrap() - escalations;
    assert!(without > 4 * bound, "without escalating, the startup work grows the WAL to {without} frames");
    assert!(escalated > 0, "the recycle escalated");
    assert!(with < without / 2, "escalating, the WAL reached {with} frames, against {without} without");
}
