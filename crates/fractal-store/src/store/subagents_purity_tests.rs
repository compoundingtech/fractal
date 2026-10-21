// A seat's open subagents fold replicated `subagent.*` claims, so every node must read the same
// subagents and leases whatever order, batching or repetition the claims arrive in. A renewal
// can arrive before its appearance, and a writer that has not yet seen an end can still renew.

use crate::store::subagents::{Scope, open_subagents_at};
use crate::store::subagents_tests_shared_seats::{SEAT, appear, claim, runtime};

fn subagents_fixture() -> &'static ReplicationExchange {
    static FIXTURE: std::sync::OnceLock<ReplicationExchange> = std::sync::OnceLock::new();
    FIXTURE.get_or_init(|| {
        let alder = Store::open_memory("alder").unwrap();
        let birch = Store::open_memory("birch").unwrap();
        let relay = Store::open_memory("cedar").unwrap();
        runtime(&alder, SEAT, "running", "inc-1");
        appear(&alder, "explore-a", 5_000).unwrap();
        appear(&alder, "explore-b", 5_000).unwrap();
        receive_and_project(
            &birch,
            "alder",
            &exchange_from(&alder, &ReplicationInventory::default()),
        );
        // alder renews one and ends the other; birch, before it sees that end, renews both and
        // starts a third.
        claim(
            &alder,
            "subagent.renewed",
            "explore-a",
            &[("lease_expires_at_unix_ms", json!(9_000))],
        )
        .unwrap();
        claim(
            &alder,
            "subagent.ended",
            "explore-b",
            &[("outcome", json!("completed"))],
        )
        .unwrap();
        claim(
            &birch,
            "subagent.renewed",
            "explore-a",
            &[("lease_expires_at_unix_ms", json!(7_000))],
        )
        .unwrap();
        claim(
            &birch,
            "subagent.renewed",
            "explore-b",
            &[("lease_expires_at_unix_ms", json!(8_000))],
        )
        .unwrap();
        appear(&birch, "explore-c", 6_000).unwrap();
        for (name, source) in [("alder", &alder), ("birch", &birch)] {
            let exchange = exchange_from(source, &ReplicationInventory::default());
            receive_and_project(&relay, name, &exchange);
        }
        exchange_from(&relay, &ReplicationInventory::default())
    })
}

fn subagents_snapshot(store: &Store) -> Value {
    let connection = store.readers.get();
    json!({
        "seat": store.open_subagents(SEAT).unwrap(),
        "running": open_subagents_at(&connection, Scope::Running(6_500)).unwrap(),
        "all": open_subagents_at(&connection, Scope::All).unwrap(),
    })
}

fn deliver_subagents(target: &Store, order: &[usize], width: usize, copies: usize) {
    let exchange = subagents_fixture();
    for chunk in order.chunks(width) {
        let envelopes = chunk
            .iter()
            .flat_map(|&index| std::iter::repeat_n(exchange.envelopes[index].clone(), copies))
            .collect();
        let admission = receive_and_project(target, "cedar", &exchange_of("cedar", envelopes));
        assert_eq!((admission.invalid, admission.unknown), (0, 0));
    }
}

fn subagents_reference() -> &'static Value {
    static REFERENCE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    REFERENCE.get_or_init(|| {
        let target = Store::open_memory("dogwood").unwrap();
        let count = subagents_fixture().envelopes.len();
        deliver_subagents(&target, &(0..count).collect::<Vec<_>>(), usize::MAX, 1);
        subagents_snapshot(&target)
    })
}

#[test]
fn the_subagents_fixture_ends_renews_and_starts_subagents() {
    let seat = subagents_reference()["seat"].as_array().unwrap();
    let open = seat
        .iter()
        .map(|subagent| {
            (
                subagent["subagent_id"].as_str().unwrap(),
                subagent["lease_expires_at_unix_ms"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    // The ended subagent stays ended despite a later renewal; the highest lease wins.
    assert_eq!(open, [("explore-a", 9_000), ("explore-c", 6_000)]);
}

#[test]
fn open_subagents_survive_redelivery_restart_and_replay() {
    let exchange = subagents_fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let target = Store::open(&path, "dogwood").unwrap();
    deliver_subagents(
        &target,
        &(0..exchange.envelopes.len()).rev().collect::<Vec<_>>(),
        1,
        1,
    );
    assert_eq!(&subagents_snapshot(&target), subagents_reference());
    receive_and_project(&target, "cedar", exchange);
    assert_eq!(&subagents_snapshot(&target), subagents_reference());
    drop(target);
    let target = Store::open(&path, "dogwood").unwrap();
    assert_eq!(&subagents_snapshot(&target), subagents_reference());
    target.replay_replication_graph().unwrap();
    assert_eq!(&subagents_snapshot(&target), subagents_reference());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn open_subagents_are_pure_under_shuffled_split_and_duplicate_delivery(
        ranks in proptest::collection::vec(any::<u64>(), 32),
        width in 1usize..5,
        copies in 1usize..3,
    ) {
        let count = subagents_fixture().envelopes.len();
        prop_assert!(count <= ranks.len());
        let mut shuffled = (0..count).collect::<Vec<_>>();
        shuffled.sort_by_key(|&index| (ranks[index], index));
        let target = Store::open_memory("dogwood").unwrap();
        deliver_subagents(&target, &shuffled, width, copies);
        prop_assert_eq!(&subagents_snapshot(&target), subagents_reference());
    }
}
