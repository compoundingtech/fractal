// Account windows fold replicated observer claims independently of delivery order,
// batching and repetition. Canonical claim order breaks equal measurement times.

const READING_SEAT: &str = "agent/alder.worker";
const BINDING_SEAT: &str = "agent/birch.worker";

fn write_limits(store: &Store, origin: &str, seat: &str, account: &str, five_hour: f64) {
    // Every claim is dated in the same millisecond, each in its own batch.
    store.set_write_clock_at(1_700_000_000_000).unwrap();
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    let body = json!({"fields": {"readings": [
        {"provider":"anthropic", "account":account, "account_ref":account, "window":"weekly",
         "used_percent":50.0, "measured_at":"2023-11-14T22:13:20Z",
         "resets_at":"2027-01-15T08:00:00Z", "coverage":"reported",
         "source":"omp-usage", "host":origin},
        {"provider":"anthropic", "account":account, "account_ref":account, "window":"5h",
         "used_percent":five_hour, "measured_at":"2023-11-14T22:13:20Z",
         "resets_at":"2027-01-15T08:00:00Z", "coverage":"reported",
         "source":"omp-usage", "host":origin}
    ], "credit_readings":[]}});
    append_claim_tx(
        &tx,
        origin,
        seat,
        "usage.limits.observed",
        Some(seat),
        &body,
        &[],
        None,
    )
    .unwrap();
    tx.commit().unwrap();
}

fn limits_fixture() -> &'static ReplicationExchange {
    static FIXTURE: std::sync::OnceLock<ReplicationExchange> = std::sync::OnceLock::new();
    FIXTURE.get_or_init(|| {
        let alder = Store::open_memory("alder").unwrap();
        let birch = Store::open_memory("birch").unwrap();
        let relay = Store::open_memory("cedar").unwrap();
        // Equal-time observations compete independently for each account window.
        write_limits(&alder, "alder", READING_SEAT, "claude/one", 10.0);
        write_limits(&birch, "birch", READING_SEAT, "claude/one", 30.0);
        // Independent provider accounts remain visible without a seat binding.
        write_limits(&alder, "alder", BINDING_SEAT, "claude/two", 20.0);
        write_limits(&birch, "birch", BINDING_SEAT, "claude/three", 20.0);
        for (name, source) in [("alder", &alder), ("birch", &birch)] {
            let exchange = exchange_from(source, &ReplicationInventory::default());
            receive_and_project(&relay, name, &exchange);
        }
        exchange_from(&relay, &ReplicationInventory::default())
    })
}

fn limits_snapshot(store: &Store) -> Value {
    json!(super::limits::account_limits_for_writers(
        &store.readers.get(), &std::collections::BTreeSet::from([
            READING_SEAT.into(), BINDING_SEAT.into(),
        ]),
    ).unwrap())
}

fn deliver(target: &Store, order: &[usize], width: usize, copies: usize) {
    let exchange = limits_fixture();
    for chunk in order.chunks(width) {
        let envelopes = chunk
            .iter()
            .flat_map(|&index| std::iter::repeat_n(exchange.envelopes[index].clone(), copies))
            .collect();
        let admission = receive_and_project(target, "cedar", &exchange_of("cedar", envelopes));
        assert_eq!((admission.invalid, admission.unknown), (0, 0));
    }
}

fn limits_reference() -> &'static Value {
    static REFERENCE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    REFERENCE.get_or_init(|| {
        let target = Store::open_memory("dogwood").unwrap();
        deliver(
            &target,
            &(0..limits_fixture().envelopes.len()).collect::<Vec<_>>(),
            usize::MAX,
            1,
        );
        limits_snapshot(&target)
    })
}

#[test]
fn every_tie_in_the_limits_fixture_is_present() {
    let accounts = limits_reference().as_array().unwrap();
    for account in ["claude/one", "claude/two", "claude/three"] {
        let limit = accounts.iter().find(|limit| limit["account"] == account).unwrap();
        assert_eq!(limit["windows"].as_array().unwrap().len(), 2);
        assert!(limit["seats"].as_array().unwrap().is_empty());
    }
}

#[test]
fn account_limits_survive_redelivery_restart_and_refold() {
    let exchange = limits_fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let target = Store::open(&path, "dogwood").unwrap();
    deliver(
        &target,
        &(0..exchange.envelopes.len()).rev().collect::<Vec<_>>(),
        1,
        1,
    );
    assert_eq!(&limits_snapshot(&target), limits_reference());
    receive_and_project(&target, "cedar", exchange);
    assert_eq!(&limits_snapshot(&target), limits_reference());
    drop(target);
    let target = Store::open(&path, "dogwood").unwrap();
    assert_eq!(&limits_snapshot(&target), limits_reference());
    {
        let mut connection = target.connection.write();
        let tx = connection.transaction().unwrap();
        super::limits::refold_limits_tx(&tx).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(&limits_snapshot(&target), limits_reference());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn account_limits_are_pure_under_shuffled_split_and_duplicate_delivery(
        ranks in proptest::collection::vec(any::<u64>(), 8),
        width in 1usize..4,
        copies in 1usize..4,
    ) {
        let count = limits_fixture().envelopes.len();
        prop_assert!(count <= ranks.len());
        let mut shuffled = (0..count).collect::<Vec<_>>();
        shuffled.sort_by_key(|&index| (ranks[index], index));
        let target = Store::open_memory("dogwood").unwrap();
        deliver(&target, &shuffled, width, copies);
        prop_assert_eq!(&limits_snapshot(&target), limits_reference());
    }
}

