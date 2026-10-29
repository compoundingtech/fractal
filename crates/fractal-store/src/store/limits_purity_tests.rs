// Account limits fold replicated `harness.limits` claims, so every node must reach the same
// answer whatever order, batching or repetition the claims arrive in. These claims tie on
// accepted time, measured time and weekly reading: only the canonical claim order may break
// those ties, never the order a node happened to admit them in.

const READING_SEAT: &str = "agent/alder.worker";
const BINDING_SEAT: &str = "agent/birch.worker";

fn write_limits(store: &Store, origin: &str, seat: &str, account: &str, five_hour: f64) {
    // Every claim is dated in the same millisecond, each in its own batch.
    store.set_write_clock_at(1_700_000_000_000).unwrap();
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    let body = json!({"fields": {
        "driver": "claude",
        "incarnation_id": "inc-1",
        "account": account,
        "weekly_percent": 50.0,
        "five_hour_percent": five_hour,
        "weekly_resets_at_unix_ms": 1_800_000_000_000_u64,
        "measured_at_unix_ms": 1_700_000_000_000_u64,
    }});
    append_claim_tx(
        &tx,
        origin,
        seat,
        "harness.limits",
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
        // One seat's readings of one account tie on everything the reading selection compares,
        // but differ in host and five-hour level.
        write_limits(&alder, "alder", READING_SEAT, "claude/one", 10.0);
        write_limits(&birch, "birch", READING_SEAT, "claude/one", 30.0);
        // Another seat's binding ties between two accounts that nothing else decides.
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
    json!(super::limits::account_limits_at(&store.readers.get()).unwrap())
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
    let seats = |account: &str| {
        accounts
            .iter()
            .find(|limit| limit["account"] == account)
            .unwrap()["seats"]
            .clone()
    };
    // Both readings of the first account compete; one binding wins.
    assert_eq!(seats("claude/one"), json!([READING_SEAT]));
    assert_eq!(
        [seats("claude/two"), seats("claude/three")]
            .iter()
            .filter(|seats| *seats == &json!([BINDING_SEAT]))
            .count(),
        1
    );
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

#[test]
fn a_version_one_build_still_folds_into_these_tables() {
    // A version 1 build that opens this store empties the tables (its version differs) and folds
    // into them with its own statements, which name only version 1's columns.
    let store = Store::open_memory("alder").unwrap();
    let connection = store.connection.write();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h216d67c8ab86 owner=fractal_store::store::limits_purity_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO account_limit_readings
             (driver, account, account_ref, weekly, measured_at_unix_ms, subject, store_index, origin, body)
             VALUES ('claude','claude/one','',1,1,?1,1,'alder','{}')",
            [READING_SEAT],
        )
        .unwrap();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-hd38e7cfc3d89 owner=fractal_store::store::limits_purity_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO account_limit_seats
             (subject, driver, account, account_ref, accepted_at_unix_ms, store_index)
             VALUES (?1,'claude','claude/one','',1,1)
             ON CONFLICT(subject) DO UPDATE SET driver=excluded.driver,
                account=excluded.account, account_ref=excluded.account_ref,
                accepted_at_unix_ms=excluded.accepted_at_unix_ms,
                store_index=excluded.store_index
             WHERE (excluded.accepted_at_unix_ms, excluded.store_index)
                >= (accepted_at_unix_ms, store_index)",
            [READING_SEAT],
        )
        .unwrap();
}
