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
    // A node projects as it starts, so it has projected before the first exchange reaches it.
    // A store that never has falls back to one full replay of everything it then holds.
    target.project_replication_backlog().unwrap();
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
    for (unit, page) in [
        ("sealed batches", SEAL_CHUNK_BATCHES),
        ("received envelopes", REPLICATION_EXCHANGE_ENVELOPE_LIMIT),
        ("admitted envelopes", ADMISSION_CHUNK_ENVELOPES),
        ("projected claims", PROJECTION_CHUNK_CLAIMS),
    ] {
        let covered = pages.get(unit).cloned().unwrap_or_default();
        assert!(covered.iter().all(|count| *count <= page), "a hold covered more than one page of {page} {unit}: {covered:?}");
        assert_eq!(covered.iter().sum::<usize>(), CLAIMS, "{unit}: {covered:?}");
    }
    // Repairs are applied all in one hold; this catch-up carries none.
    let repairs = pages.get("applied repairs").cloned().unwrap_or_default();
    assert!(!repairs.is_empty() && repairs.iter().all(|count| *count == 0), "{repairs:?}");
}
