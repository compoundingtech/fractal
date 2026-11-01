#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::writer_bound_tests_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::writer_bound_tests_shared_missions::BOUND;
use super::*;
use std::time::Duration;
use fractal_store::store::writer_bound_tests_shared_delivery::*;
use fractal_store::store::writer_bound_tests_shared_delivery::{FLEET};
use smallclaims::windows::holds::holds_during;


const CLAIMS: usize = 1500;

/// A peer catching up on 1,500 claims: export's sealing, receipt, admission and projection each
/// hold the writer one page at a time, and every batch, envelope and claim goes through in those
/// pages. Each paged path reports what its hold covered, so this counts pages instead of timing
/// holds: a loaded host stretches a hold on the wall clock, never the page it covers.
#[test]
fn a_catch_up_exchange_holds_the_writer_one_page_at_a_time() {
    let directory = tempfile::tempdir().unwrap();
    let node = |name: &str| {
        let store = Store::open(&directory.path().join(format!("{name}.sqlite3")), name).unwrap();
        // The bound is on how much work one writer hold does. With synchronous=FULL a loaded disk's
        // fsync once made an admit hold 684 ms (13 ms unloaded); durability is not what this checks.
        store
            .connection
            .write()
            .pragma_update(None, "synchronous", "OFF")
            .unwrap();
        store.bind_fleet(FLEET).unwrap();
        store
    };
    let source = node("birch");
    let target = node("cedar");
    // The target has never projected: its first projection pages the catch-up like any other.
    let local = usize::try_from(target.index().unwrap()).unwrap();
    smallclaims::store::FULL_REPLAYS.with(|replays| replays.set(0));
    for index in 0..CLAIMS {
        source
            .append_claim(&ClaimInput {
                subject: format!("message/bound-{index}"),
                kind: "message.sent".into(),
                actor: Some("person/avery".into()),
                fields: BTreeMap::from([
                    ("from".into(), json!("person/avery")),
                    ("to".into(), json!(format!("agent/bound/{}", index % 40))),
                    ("content".into(), json!(format!("note {index} {}", "x".repeat(200)))),
                    ("status".into(), json!("sent")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    let mut pages: BTreeMap<&'static str, Vec<usize>> = BTreeMap::new();
    let mut paged = |holds: smallclaims::windows::holds::Holds| {
        for (unit, count) in holds.pages {
            pages.entry(unit).or_default().push(count);
        }
    };
    let mut sent = 0;
    for _ in 0..16 {
        let inventory = target.replication_inventory().unwrap();
        let (exchange, holds) = holds_during(|| source.export_replication_exchange(FLEET, &inventory).unwrap());
        paged(holds);
        if exchange.envelopes.is_empty() {
            break;
        }
        sent += exchange.envelopes.len();
        paged(holds_during(|| target.receive_replication_exchange("birch", FLEET, &exchange).unwrap()).1);
        paged(holds_during(|| target.validate_replication_backlog().unwrap()).1);
        paged(holds_during(|| target.apply_replication_repairs().unwrap()).1);
        paged(holds_during(|| target.project_replication_backlog().unwrap()).1);
    }
    assert_eq!(
        target.replication_inventory().unwrap().digest,
        source.replication_inventory().unwrap().digest,
        "the catch-up finished"
    );
    assert_eq!(sent, CLAIMS, "one envelope for each claim's batch");
    // Each paged hold covers at most one page, and together they cover everything once.
    use smallclaims::store::{prepared_seal::SEAL_CHUNK_BATCHES, ADMISSION_CHUNK_ENVELOPES, PROJECTION_CHUNK_CLAIMS,
        REPLICATION_EXCHANGE_ENVELOPE_LIMIT};
    for (unit, page, all) in [
        ("sealed batches", SEAL_CHUNK_BATCHES, CLAIMS),
        ("received envelopes", REPLICATION_EXCHANGE_ENVELOPE_LIMIT, CLAIMS),
        ("admitted envelopes", ADMISSION_CHUNK_ENVELOPES, CLAIMS),
        // With the target's own claims, written before its first projection.
        ("projected claims", PROJECTION_CHUNK_CLAIMS, CLAIMS + local),
    ] {
        let covered = pages.get(unit).cloned().unwrap_or_default();
        assert!(covered.iter().all(|count| *count <= page), "a hold covered more than one page of {page} {unit}: {covered:?}");
        assert_eq!(covered.iter().sum::<usize>(), all, "{unit}: {covered:?}");
    }
    assert_eq!(smallclaims::store::FULL_REPLAYS.with(std::cell::Cell::get), 0);
    // This catch-up carries no repairs, so applying them asks for no writer hold.
    assert_eq!(pages.get("applied repairs"), None);
}

/// A store that has never projected receives a backlog, then stops partway through its first
/// projection, as a crash between pages would. Reopened, it projects the rest a page per hold
/// from where the stopped pass committed, and the result is what a full replay makes.
#[test]
fn a_never_projected_store_pages_its_first_projection_and_resumes_after_a_stop() {
    use smallclaims::store::{FULL_REPLAYS, PROJECTION_CHUNK_CLAIMS};
    let directory = tempfile::tempdir().unwrap();
    let open = |name: &str| {
        let store = Store::open(&directory.path().join(format!("{name}.sqlite3")), name).unwrap();
        store.connection.write().pragma_update(None, "synchronous", "OFF").unwrap();
        store
    };
    let source = open("birch");
    source.bind_fleet(FLEET).unwrap();
    let target = open("cedar");
    target.bind_fleet(FLEET).unwrap();
    for index in 0..600 {
        source
            .append_claim(&ClaimInput {
                subject: format!("message/first-{index}"),
                kind: "message.sent".into(),
                actor: Some("person/avery".into()),
                fields: BTreeMap::from([
                    ("from".into(), json!("person/avery")),
                    ("to".into(), json!(format!("agent/first/{}", index % 7))),
                    ("content".into(), json!(format!("note {index}"))),
                    ("status".into(), json!("sent")),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    loop {
        let exchange = source
            .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
            .unwrap();
        if exchange.envelopes.is_empty() {
            break;
        }
        target.receive_replication_exchange("birch", FLEET, &exchange).unwrap();
        target.validate_replication_backlog().unwrap();
    }
    let all = target.index().unwrap();
    let frontier = |store: &Store| -> u64 {
        store
            .readers
            .get()
            .query_row(
                "SELECT last_good_store_index FROM projection_health WHERE aggregate='graph'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(frontier(&target), 0, "nothing projected yet");
    FULL_REPLAYS.with(|replays| replays.set(0));
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        target
            .project_replication_backlog_with_yield(|| panic!("stopped between pages"))
            .unwrap()
    }));
    assert!(stopped.is_err());
    let first = frontier(&target);
    assert!(first > 0 && first < all, "one page committed before the stop: {first} of {all}");

    drop(target);
    let target = open("cedar");
    assert_eq!(frontier(&target), first, "the committed page survives the restart");
    let ((), holds) = holds_during(|| {
        target.project_replication_backlog().unwrap();
    });
    let pages = holds.pages_of("projected claims");
    assert!(pages.len() > 1 && pages.iter().all(|count| *count <= PROJECTION_CHUNK_CLAIMS), "{pages:?}");
    assert_eq!(pages.iter().sum::<usize>() as u64, all - first, "the rest, once: {pages:?}");
    assert_eq!(frontier(&target), all);
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0, "no pass replayed from nothing");

    let paged = target.replication_status(true, Some(FLEET), &[]).unwrap().graph_digest;
    target.replay_replication_graph().unwrap();
    assert_eq!(paged, target.replication_status(true, Some(FLEET), &[]).unwrap().graph_digest,
        "the paged projection matches a full replay");
}

/// Measures a never-projected store's first projection of a backlog it received from a private
/// copy of a member store: how many claims each writer hold covered, and the longest hold.
#[test]
#[ignore = "requires ST_FIRST_PROJECTION_BENCH_STORE, a private writable store copy"]
fn first_projection_hold_benchmark() {
    let path = std::path::PathBuf::from(std::env::var("ST_FIRST_PROJECTION_BENCH_STORE").unwrap());
    let exchanges: usize = std::env::var("ST_FIRST_PROJECTION_BENCH_EXCHANGES")
        .map_or(8, |count| count.parse().unwrap());
    let source = Store::open(&path, "first-projection-source").unwrap();
    let fleet: String = source
        .readers
        .get()
        .query_row("SELECT value FROM meta WHERE key='fleet_id'", [], |row| row.get(0))
        .unwrap();
    let directory = tempfile::tempdir_in(path.parent().unwrap()).unwrap();
    let target = Store::open(&directory.path().join("target.sqlite3"), "first-projection-target").unwrap();
    target.bind_fleet(&fleet).unwrap();
    for _ in 0..exchanges {
        let mut inventory = target.export_replication_summary(&fleet).unwrap().inventory;
        if inventory.buckets.is_empty() {
            // An empty store's summary has no range digests, and a source answers a bucketless
            // inventory with its whole inventory, past the metadata limit at this size (see
            // #3440). One empty range asks for a compact answer instead.
            inventory.buckets.push(smallclaims::replication::ReplicationInventoryBucket {
                writer: String::new(),
                start: 0,
                count: 0,
                digest: String::new(),
            });
        }
        let exchange = source.export_replication_exchange(&fleet, &inventory).unwrap();
        if exchange.envelopes.is_empty() {
            break;
        }
        target.receive_replication_exchange("first-projection-source", &fleet, &exchange).unwrap();
        target.validate_replication_backlog().unwrap();
    }
    let started = std::time::Instant::now();
    let ((), holds) = holds_during(|| {
        target.project_replication_backlog().unwrap();
        // The writer thread ends the last hold after the connection comes back to it.
        std::thread::sleep(Duration::from_millis(100));
    });
    eprintln!("{}", json!({
        "claims": target.index().unwrap(),
        "pages": holds.pages_of("projected claims"),
        "longest_hold_us": holds.longest.as_micros(),
        "call_us": started.elapsed().as_micros() - 100_000,
    }));
}
