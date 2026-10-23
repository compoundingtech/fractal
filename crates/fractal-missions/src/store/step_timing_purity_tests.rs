// A step's execution clock folds replicated step and work claims, so every node must read the
// same running interval, elapsed time, wait and lease whatever order, batching or repetition the
// claims arrive in. Two writers record events in the same millisecond, one of them a pause.

const TIMED_STEP: &str = "step-run/timing-run/build";
const T0: u128 = 1_700_000_000_000;

fn timing_claim(store: &Store, at: u128, kind: &str, fields: Value) {
    store.set_write_clock_at(at).unwrap();
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    append_claim_tx(
        &tx,
        &store.origin,
        TIMED_STEP,
        kind,
        None,
        &json!({ "fields": fields }),
        &[],
        None,
    )
    .unwrap();
    tx.commit().unwrap();
}

fn work_fields(expires: Option<u128>, extra: Value) -> Value {
    let mut fields = json!({"attempt": 1, "status": "working", "claimant": "agent/alder.worker"});
    if let Some(expires) = expires {
        fields["claim_expires_at_unix_ms"] = json!(expires as u64);
    }
    for (name, value) in extra.as_object().unwrap() {
        fields[name] = value.clone();
    }
    fields
}

fn timing_fixture() -> &'static ReplicationExchange {
    static FIXTURE: std::sync::OnceLock<ReplicationExchange> = std::sync::OnceLock::new();
    FIXTURE.get_or_init(|| {
        let alder = Store::open_memory("alder").unwrap();
        let birch = Store::open_memory("birch").unwrap();
        let relay = Store::open_memory("cedar").unwrap();
        timing_claim(
            &alder,
            T0,
            "work.claimed",
            work_fields(Some(T0 + 30_000), json!({})),
        );
        timing_claim(
            &alder,
            T0 + 20_000,
            "work.renewed",
            work_fields(Some(T0 + 60_000), json!({})),
        );
        // A pause in the same millisecond as the renewal, from another writer.
        timing_claim(
            &birch,
            T0 + 20_000,
            "work.progress",
            work_fields(
                Some(T0 + 60_000),
                json!({"clock": "paused", "wait_until_unix_ms": (T0 + 50_000) as u64}),
            ),
        );
        timing_claim(
            &alder,
            T0 + 40_000,
            "work.progress",
            work_fields(Some(T0 + 90_000), json!({"clock": "running"})),
        );
        timing_claim(
            &birch,
            T0 + 40_000,
            "work.renewed",
            work_fields(Some(T0 + 80_000), json!({})),
        );
        // After the lease ran out: the interval closed and this renewal opens another.
        timing_claim(
            &alder,
            T0 + 120_000,
            "work.renewed",
            work_fields(Some(T0 + 170_000), json!({})),
        );
        timing_claim(
            &alder,
            T0 + 150_000,
            "work.submitted",
            work_fields(None, json!({})),
        );
        for (name, source) in [("alder", &alder), ("birch", &birch)] {
            let exchange = exchange_from(source, &ReplicationInventory::default());
            receive_and_project(&relay, name, &exchange);
        }
        exchange_from(&relay, &ReplicationInventory::default())
    })
}

fn timing_snapshot(store: &Store) -> Value {
    let connection = store.readers.get();
    let mut reads = Vec::new();
    for offset in [10_000, 25_000, 45_000, 100_000, 130_000, 200_000] {
        for active in [true, false] {
            let clock =
                step_execution_clock_at(&connection, TIMED_STEP, 1, T0 + offset, active).unwrap();
            reads.push(json!({
                "at": offset, "active": active,
                "started": clock.started.map(|at| at.to_string()),
                "elapsed": clock.elapsed.to_string(),
                "paused_until": clock.paused_until.map(|at| at.to_string()),
                "waited": clock.waited.to_string(),
                "lease": step_timing_lease_at(&connection, TIMED_STEP, 1, T0 + offset).unwrap()
                    .map(|at| at.to_string()),
            }));
        }
    }
    json!(reads)
}

fn deliver_timing(target: &Store, order: &[usize], width: usize, copies: usize) {
    let exchange = timing_fixture();
    for chunk in order.chunks(width) {
        let envelopes = chunk
            .iter()
            .flat_map(|&index| std::iter::repeat_n(exchange.envelopes[index].clone(), copies))
            .collect();
        let admission = receive_and_project(target, "cedar", &exchange_of("cedar", envelopes));
        assert_eq!((admission.invalid, admission.unknown), (0, 0));
    }
}

fn timing_reference() -> &'static Value {
    static REFERENCE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    REFERENCE.get_or_init(|| {
        let target = Store::open_memory("dogwood").unwrap();
        let count = timing_fixture().envelopes.len();
        deliver_timing(&target, &(0..count).collect::<Vec<_>>(), usize::MAX, 1);
        timing_snapshot(&target)
    })
}

#[test]
fn the_timing_fixture_pauses_and_reopens_the_clock() {
    let reads = timing_reference().as_array().unwrap();
    let at = |offset: u64, active: bool| {
        reads
            .iter()
            .find(|read| read["at"] == offset && read["active"] == active)
            .unwrap()
    };
    // Paused while the wait runs, running again after the reopened interval starts.
    assert!(at(25_000, true)["paused_until"].is_string());
    assert!(at(45_000, true)["started"].is_string());
    assert_ne!(at(130_000, true)["started"], at(45_000, true)["started"]);
}

#[test]
fn step_timing_survives_redelivery_restart_and_replay() {
    let exchange = timing_fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let target = Store::open(&path, "dogwood").unwrap();
    deliver_timing(
        &target,
        &(0..exchange.envelopes.len()).rev().collect::<Vec<_>>(),
        1,
        1,
    );
    assert_eq!(&timing_snapshot(&target), timing_reference());
    receive_and_project(&target, "cedar", exchange);
    assert_eq!(&timing_snapshot(&target), timing_reference());
    drop(target);
    let target = Store::open(&path, "dogwood").unwrap();
    assert_eq!(&timing_snapshot(&target), timing_reference());
    target.replay_replication_graph().unwrap();
    assert_eq!(&timing_snapshot(&target), timing_reference());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn step_timing_is_pure_under_shuffled_split_and_duplicate_delivery(
        ranks in proptest::collection::vec(any::<u64>(), 16),
        width in 1usize..4,
        copies in 1usize..3,
    ) {
        let count = timing_fixture().envelopes.len();
        prop_assert!(count <= ranks.len());
        let mut shuffled = (0..count).collect::<Vec<_>>();
        shuffled.sort_by_key(|&index| (ranks[index], index));
        let target = Store::open_memory("dogwood").unwrap();
        deliver_timing(&target, &shuffled, width, copies);
        prop_assert_eq!(&timing_snapshot(&target), timing_reference());
    }
}
