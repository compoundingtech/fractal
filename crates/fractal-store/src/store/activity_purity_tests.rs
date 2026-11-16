const AGENT: &str = "agent/alder.worker";
const INCARNATION: &str = "activity-test";

fn activity_source(category: usize, times: &[u128]) -> Store {
    let source = Store::open_memory("alder").unwrap();
    source.set_write_clock_at(0).unwrap();
    for (kind, fields) in [
        (
            "runtime.observed",
            json!({"status": "running", "incarnation_id": INCARNATION}),
        ),
        (
            "harness.observed",
            json!({"state": "idle", "driver": "codex", "incarnation_id": INCARNATION}),
        ),
    ] {
        let mut connection = source.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_tx(
            &tx,
            &source.origin,
            AGENT,
            kind,
            Some(AGENT),
            &json!({"fields": fields}),
            &[],
            None,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    for (index, &time) in times.iter().enumerate() {
        source.set_write_clock_at(time).unwrap();
        let (subject, kind, fields) = match category {
            0 => (
                format!("message/activity-{index}"),
                "message.sent",
                json!({"from": AGENT, "to": "agent/birch.worker", "content": "Progress", "status": "sent"}),
            ),
            1 => (
                format!("message/activity-{index}"),
                "message.sent",
                json!({"from": "agent/birch.worker", "to": AGENT, "content": "Progress", "status": "sent"}),
            ),
            2 => (
                "step-run/activity/work".into(),
                "work.progress",
                json!({"summary": "Progress"}),
            ),
            3 => (
                format!("step-run/activity/work-{index}"),
                "work.submitted",
                json!({"status": "completed", "summary": "Done"}),
            ),
            _ => (
                AGENT.into(),
                "harness.timeline",
                json!({
                    "incarnation_id": INCARNATION, "driver": "codex", "entry_id": format!("entry-{index}"),
                    "entry_type": "message", "operation": "append", "revision": 1,
                    "role": "assistant", "final": true, "body": {"text": "Progress"}
                }),
            ),
        };
        // Internal append also covers legacy durable timelines; native timeline writes
        // now stay local. Replicas use ordinary envelope/schema admission below.
        let mut connection = source.connection.write();
        let tx = connection.transaction().unwrap();
        append_claim_tx(
            &tx,
            &source.origin,
            &subject,
            kind,
            Some(AGENT),
            &json!({"fields": fields}),
            &[],
            None,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    source
}

// Pull requests run 8 cases from a fixed seed, so a failure reproduces and every PR checks the same
// permutations; that seed's 8 cases cover all 5 categories. The daily run (ST_TEST_SCALE=full, docs/ci.md)
// keeps the 32 random cases this test always ran.
fn purity_config() -> ProptestConfig {
    if std::env::var_os("ST_TEST_SCALE").is_some_and(|scale| scale == "full") {
        ProptestConfig::with_cases(32)
    } else {
        ProptestConfig {
            cases: 8,
            rng_seed: proptest::test_runner::RngSeed::Fixed(PURITY_SEED),
            ..ProptestConfig::default()
        }
    }
}

// Chosen because its 8 cases cover categories 0-4 (checked by printing each case's category).
const PURITY_SEED: u64 = 2;

proptest! {
    #![proptest_config(purity_config())]

    #[test]
    fn last_activity_is_pure_under_shuffled_split_and_duplicate_delivery(
        category in 0usize..5,
        times in proptest::collection::vec(1u64..1_000_000, 2..7),
        ranks in proptest::collection::vec(any::<u64>(), 8),
        width in 1usize..7,
        copies in 1usize..4,
    ) {
        let mut times = times;
        times.sort();
        let source = activity_source(category, &times.iter().map(|&time| time as u128).collect::<Vec<_>>());
        let exchange = exchange_from(&source, &ReplicationInventory::default());
        prop_assert_eq!(exchange.envelopes.len(), times.len() + 2);
        let expected = Some(*times.last().unwrap() as u128);
        let mut order = (0..exchange.envelopes.len()).collect::<Vec<_>>();
        order.sort_by_key(|&index| (ranks[index], index));
        for schedule in [
            (0..exchange.envelopes.len()).rev().collect::<Vec<_>>(),
            order,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("claims.sqlite3");
            let target = Store::open(&path, "cedar").unwrap();
            for chunk in schedule.chunks(width) {
                let envelopes = chunk.iter().flat_map(|&index| {
                    std::iter::repeat_n(exchange.envelopes[index].clone(), copies)
                }).collect();
                let admitted = receive_and_project(&target, "alder", &exchange_of("alder", envelopes));
                prop_assert_eq!(admitted.invalid, 0);
                prop_assert_eq!(admitted.unknown, 0);
            }
            let index = target.index().unwrap();
            prop_assert_eq!(target.agent_last_activity_at(AGENT, Some(INCARNATION), index).unwrap(), expected);
            let cards = target.agent_card_reads(&[AGENT.into()], index, &HashMap::new()).unwrap();
            prop_assert_eq!(cards.last_activity_at(&target, AGENT, Some(INCARNATION)).unwrap(), expected);
            receive_and_project(&target, "alder", &exchange);
            let index = target.index().unwrap();
            prop_assert_eq!(target.agent_last_activity_at(AGENT, Some(INCARNATION), index).unwrap(), expected);
            let cards = target.agent_card_reads(&[AGENT.into()], index, &HashMap::new()).unwrap();
            prop_assert_eq!(cards.last_activity_at(&target, AGENT, Some(INCARNATION)).unwrap(), expected);
            target.replay_replication_graph().unwrap();
            let index = target.index().unwrap();
            prop_assert_eq!(target.agent_last_activity_at(AGENT, Some(INCARNATION), index).unwrap(), expected);
            let cards = target.agent_card_reads(&[AGENT.into()], index, &HashMap::new()).unwrap();
            prop_assert_eq!(cards.last_activity_at(&target, AGENT, Some(INCARNATION)).unwrap(), expected);
            // A reopened store reads the same.
            drop(target);
            let target = Store::open(&path, "cedar").unwrap();
            let index = target.index().unwrap();
            prop_assert_eq!(target.agent_last_activity_at(AGENT, Some(INCARNATION), index).unwrap(), expected);
            let cards = target.agent_card_reads(&[AGENT.into()], index, &HashMap::new()).unwrap();
            prop_assert_eq!(cards.last_activity_at(&target, AGENT, Some(INCARNATION)).unwrap(), expected);
        }
    }
}

#[test]
fn last_activity_uses_numeric_time_and_preserves_historical_frontiers() {
    for category in 0..5 {
        let times = [9, 10, 11];
        let source = activity_source(category, &times);
        let exchange = exchange_from(&source, &ReplicationInventory::default());
        let target = Store::open_memory("cedar").unwrap();
        let mut envelopes = exchange.envelopes.clone();
        envelopes.sort_by_key(|envelope| envelope.accepted_at_unix_ms);
        let mut snapshots = vec![(0, None)];
        for envelope in envelopes {
            let time = envelope.accepted_at_unix_ms;
            receive_and_project(&target, "alder", &exchange_of("alder", vec![envelope]));
            snapshots.push((target.index().unwrap(), (time > 0).then_some(time)));
        }
        // The local test clock accepts signed milliseconds. Use a synthetic accepted-time
        // row for the wider reader domain, separately from the envelope admission proof.
        let wide = 1u128 << 127;
        target
            .connection
            .write()
            .execute(
                /* sqlite-large-value: allow-for-now lv-h2409bb4f3cb4 owner=fractal_store::store::activity_purity_tests TODO=prepare-body-and-predecessor-values */ "UPDATE claims SET accepted_at_unix_ms=?1 WHERE accepted_at_unix_ms='11'",
                [wide.to_string()],
            )
            .unwrap();
        snapshots.last_mut().unwrap().1 = Some(wide);
        // Re-read old frontiers after later claims have arrived. A time index must not
        // let the newest accepted claim escape the caller's snapshot membership.
        for (index, expected) in snapshots {
            assert_eq!(
                target
                    .agent_last_activity_at(AGENT, Some(INCARNATION), index)
                    .unwrap(),
                expected
            );
            let cards = target
                .agent_card_reads(&[AGENT.into()], index, &HashMap::new())
                .unwrap();
            assert_eq!(
                cards
                    .last_activity_at(&target, AGENT, Some(INCARNATION))
                    .unwrap(),
                expected
            );
        }
    }
}

#[test]
fn last_activity_reads_do_not_scan_growing_completed_history() {
    use smallclaims::sqlite::work::SqliteWorkScope;
    let measure = |category, count| {
        let times = (1..=count).collect::<Vec<u128>>();
        let store = activity_source(category, &times);
        let index = current_index(&store.readers.get()).unwrap();
        let scope = SqliteWorkScope::start();
        assert_eq!(
            store
                .agent_last_activity_at(AGENT, Some(INCARNATION), index)
                .unwrap(),
            Some(count)
        );
        let cards = store
            .agent_card_reads(&[AGENT.into()], index, &HashMap::new())
            .unwrap();
        assert_eq!(
            cards
                .last_activity_at(&store, AGENT, Some(INCARNATION))
                .unwrap(),
            Some(count)
        );
        let work = scope.finish();
        assert!(work.vm_steps > 0);
        work
    };
    for category in 0..5 {
        let small = measure(category, 128);
        let large = measure(category, 1024);
        assert!(
            large.vm_steps <= small.vm_steps + 100,
            "category={category}, small={small:?}, large={large:?}"
        );
        assert_eq!(
            large.fullscan_steps, 0,
            "category={category}, work={large:?}"
        );
        assert_eq!(
            large.sorts, small.sorts,
            "category={category}, small={small:?}, large={large:?}"
        );
    }
}

fn append_at(store: &Store, time: u128, subject: &str, kind: &str, fields: Value) {
    store.set_write_clock_at(time).unwrap();
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    append_claim_tx(&tx, &store.origin, subject, kind, Some(AGENT), &json!({"fields": fields}), &[], None).unwrap();
    tx.commit().unwrap();
}

fn timeline(entry: usize, entry_type: &str, role: &str) -> Value {
    json!({
        "incarnation_id": INCARNATION, "driver": "codex", "entry_id": format!("prose-{entry}"),
        "entry_type": entry_type, "operation": "append", "revision": 1,
        "sequence": entry + 1,
        "role": role, "final": true, "body": {"text": "Words"}
    })
}

/// "New since you last looked" compares with the agent's own prose: its assistant timeline text and the
/// mail it sends. Tool calls, the person's prompts and mail to the agent are activity, not prose.
#[test]
fn last_prose_is_the_agents_own_words_only() {
    let store = activity_source(4, &[]);
    let prose = |store: &Store| store.agent_last_prose_at(AGENT, Some(INCARNATION), store.index().unwrap()).unwrap();
    assert_eq!(prose(&store), None);
    append_at(&store, 100, AGENT, "harness.timeline", timeline(1, "message", "assistant"));
    append_at(&store, 200, AGENT, "harness.timeline", timeline(2, "tool_call", "assistant"));
    append_at(&store, 300, AGENT, "harness.timeline", timeline(3, "message", "user"));
    append_at(&store, 400, "message/to-agent", "message.sent", json!({"from": "person/ada", "to": AGENT, "content": "Hi", "status": "sent"}));
    assert_eq!(prose(&store), Some(100));
    assert_eq!(store.agent_last_activity_at(AGENT, Some(INCARNATION), store.index().unwrap()).unwrap(), Some(400));
    append_at(&store, 500, "message/from-agent", "message.sent", json!({"from": AGENT, "to": "person/ada", "content": "Done", "status": "sent"}));
    assert_eq!(prose(&store), Some(500));
    append_at(&store, 600, AGENT, "harness.timeline", timeline(4, "content", "assistant"));
    assert_eq!(prose(&store), Some(600));
}

#[test]
fn last_prose_reads_do_not_scan_growing_non_prose_history() {
    use smallclaims::sqlite::work::SqliteWorkScope;

    let measure = |local, count, has_prose, tools_only| {
        let store = activity_source(4, &[]);
        let append = |entry, entry_type, role, incarnation| {
            let mut fields = timeline(entry, entry_type, role);
            fields["incarnation_id"] = json!(incarnation);
            if local {
                store.append_claim(&ClaimInput {
                    subject: AGENT.into(),
                    kind: "harness.timeline".into(),
                    actor: Some(AGENT.into()),
                    fields: serde_json::from_value(fields).unwrap(),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: None,
                }).unwrap();
            } else {
                append_at(&store, entry as u128 + 1, AGENT, "harness.timeline", fields);
            }
        };
        if has_prose {
            append(0, "message", "assistant", INCARNATION);
        }
        let index = store.index().unwrap();
        let expected = store.agent_last_prose_at(AGENT, Some(INCARNATION), index).unwrap();
        assert_eq!(expected.is_some(), has_prose);
        for entry in 1..=count {
            let (entry_type, role, incarnation) = match entry % if tools_only { 2 } else { 4 } {
                0 => ("tool_call", "assistant", INCARNATION),
                1 => ("tool_result", "tool", INCARNATION),
                2 => ("message", "user", INCARNATION),
                _ => ("content", "assistant", "other-incarnation"),
            };
            append(entry, entry_type, role, incarnation);
        }
        let index = store.index().unwrap();
        let scope = SqliteWorkScope::start();
        // This is the per-card roster read, including both timeline seeks and sent mail.
        assert_eq!(
            store.agent_last_prose_at(AGENT, Some(INCARNATION), index).unwrap(),
            expected
        );
        let work = scope.finish();
        assert!(work.vm_steps > 0, "{work:?}");
        assert_eq!(work.fullscan_steps, 0, "{work:?}");
        assert_eq!(work.sorts, 0, "{work:?}");
        work
    };
    for local in [false, true] {
        for tools_only in [true, false] {
            // An absent frontier is the worst case for a residual role/type filter.
            for has_prose in [false, true] {
                let small = measure(local, 128, has_prose, tools_only);
                let large = measure(local, 4096, has_prose, tools_only);
                assert_eq!(large.statements, small.statements);
                assert!(
                    large.vm_steps <= small.vm_steps + 100,
                    "local={local}, has_prose={has_prose}, tools_only={tools_only}, small={small:?}, large={large:?}"
                );
            }
        }
    }
}
