// A seat's queue order folds replicated `agent.queue.moved` claims over the runs that joined it,
// so every node must read the same order whatever order, batching or repetition the claims
// arrive in. Two writers move runs in the same millisecond: only the canonical claim order may
// decide which move applies last.

const SEAT: &str = "agent/garden/ash";

fn queue_mission(store: &Store) {
    publish_mission(
        store,
        &format!(
            r#"version 2
mission "garden/tend" state="ready" {{
  goal "Tend one seat."
  concurrent-runs max=8
  step "plant" {{ assigned-to "{SEAT}" }}
}}
"#
        ),
        "publish-garden-tend",
    );
}

fn queue_run(store: &Store, key: &str) -> String {
    store
        .create_mission_run(&MissionRunRequest {
            mission: "garden/tend".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: key.into(),
        })
        .unwrap()
        .id
}

fn queue_move(store: &Store, run: &str, placement: &str, anchor: Option<&str>, key: &str) {
    store
        .move_seat_queue_run(&SeatQueueMoveRequest {
            agent: SEAT.into(),
            run: run.into(),
            placement: placement.into(),
            anchor: anchor.map(str::to_owned),
            reason: None,
            actor: "person/operator".into(),
            idempotency_key: key.into(),
        })
        .unwrap();
}

fn queue_fixture() -> &'static (ReplicationExchange, Vec<String>) {
    static FIXTURE: std::sync::OnceLock<(ReplicationExchange, Vec<String>)> =
        std::sync::OnceLock::new();
    FIXTURE.get_or_init(|| {
        let alder = Store::open_memory("alder").unwrap();
        let birch = Store::open_memory("birch").unwrap();
        let relay = Store::open_memory("cedar").unwrap();
        alder.set_write_clock_at(1_700_000_000_000).unwrap();
        queue_mission(&alder);
        let runs = (0..3)
            .map(|n| {
                alder
                    .set_write_clock_at(1_700_000_001_000 + n * 1_000)
                    .unwrap();
                queue_run(&alder, &format!("tend-{n}"))
            })
            .collect::<Vec<_>>();
        receive_and_project(
            &birch,
            "alder",
            &exchange_from(&alder, &ReplicationInventory::default()),
        );
        // Both writers move a run in the same millisecond, and each names the other's run.
        alder.set_write_clock_at(1_700_000_010_000).unwrap();
        queue_move(&alder, &runs[2], "top", None, "alder-top");
        birch.set_write_clock_at(1_700_000_010_000).unwrap();
        queue_move(&birch, &runs[0], "before", Some(&runs[1]), "birch-before");
        birch.set_write_clock_at(1_700_000_010_000).unwrap();
        queue_move(&birch, &runs[1], "top", None, "birch-top");
        for (name, source) in [("alder", &alder), ("birch", &birch)] {
            let exchange = exchange_from(source, &ReplicationInventory::default());
            receive_and_project(&relay, name, &exchange);
        }
        (
            exchange_from(&relay, &ReplicationInventory::default()),
            runs,
        )
    })
}

fn queue_snapshot(store: &Store) -> Value {
    json!({
        "one": store.seat_run_order(SEAT).unwrap(),
        "every": store.seat_run_orders().unwrap(),
    })
}

fn deliver_queue(target: &Store, order: &[usize], width: usize, copies: usize) {
    let (exchange, _) = queue_fixture();
    for chunk in order.chunks(width) {
        let envelopes = chunk
            .iter()
            .flat_map(|&index| std::iter::repeat_n(exchange.envelopes[index].clone(), copies))
            .collect();
        let admission = receive_and_project(target, "cedar", &exchange_of("cedar", envelopes));
        assert_eq!((admission.invalid, admission.unknown), (0, 0));
    }
}

fn queue_reference() -> &'static Value {
    static REFERENCE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    REFERENCE.get_or_init(|| {
        let target = Store::open_memory("dogwood").unwrap();
        let count = queue_fixture().0.envelopes.len();
        deliver_queue(&target, &(0..count).collect::<Vec<_>>(), usize::MAX, 1);
        queue_snapshot(&target)
    })
}

#[test]
fn the_queue_fixture_moves_every_run() {
    let (_, runs) = queue_fixture();
    // Without its moves the queue would read in join order.
    assert_eq!(
        queue_reference()["one"].as_array().unwrap().len(),
        runs.len()
    );
    assert_ne!(queue_reference()["one"], json!(runs));
}

#[test]
fn seat_queue_order_survives_redelivery_restart_and_replay() {
    let (exchange, _) = queue_fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claims.sqlite3");
    let target = Store::open(&path, "dogwood").unwrap();
    deliver_queue(
        &target,
        &(0..exchange.envelopes.len()).rev().collect::<Vec<_>>(),
        1,
        1,
    );
    assert_eq!(&queue_snapshot(&target), queue_reference());
    receive_and_project(&target, "cedar", exchange);
    assert_eq!(&queue_snapshot(&target), queue_reference());
    drop(target);
    let target = Store::open(&path, "dogwood").unwrap();
    assert_eq!(&queue_snapshot(&target), queue_reference());
    target.replay_replication_graph().unwrap();
    assert_eq!(&queue_snapshot(&target), queue_reference());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn seat_queue_order_is_pure_under_shuffled_split_and_duplicate_delivery(
        ranks in proptest::collection::vec(any::<u64>(), 32),
        width in 1usize..5,
        copies in 1usize..3,
    ) {
        let count = queue_fixture().0.envelopes.len();
        prop_assert!(count <= ranks.len());
        let mut shuffled = (0..count).collect::<Vec<_>>();
        shuffled.sort_by_key(|&index| (ranks[index], index));
        let target = Store::open_memory("dogwood").unwrap();
        deliver_queue(&target, &shuffled, width, copies);
        prop_assert_eq!(&queue_snapshot(&target), queue_reference());
    }
}
