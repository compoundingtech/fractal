//! A one-field status read of a seat costs the same however many claims the seat has. Drivers
//! and `st work` read a seat's incarnation from its status, and the full reduction folded every
//! runtime observation of the seat and listed every claim id: seconds on a busy host's store.
use super::*;
use serde_json::json;

const SEAT: &str = "agent/example/worker";

fn observe(store: &Store, fields: serde_json::Value) {
    store
        .append_claim(&crate::model::ClaimInput {
            subject: SEAT.into(),
            kind: "runtime.observed".into(),
            actor: None,
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn seat_with(observations: usize) -> Store {
    let store = Store::open_memory("node").unwrap();
    for index in 0..observations {
        observe(&store, json!({
            "status": if index % 2 == 0 { "running" } else { "stopped" },
            "incarnation_id": format!("incarnation-{index}"),
            "runtime_id": format!("runtime-{index}"),
        }));
    }
    store
}

fn work(read: impl FnOnce() -> StatusResponse) -> (smallclaims::sqlite::work::SqliteWork, StatusResponse) {
    let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let status = read();
    (scope.finish(), status)
}

#[test]
fn a_summary_status_read_of_a_seat_does_not_read_its_history() {
    let (few, many) = (seat_with(50), seat_with(3_000));
    let (few_summary, _) = work(|| few.status_summary_at(SEAT, None).unwrap());
    let (many_summary, summary) = work(|| many.status_summary_at(SEAT, None).unwrap());
    let (few_full, _) = work(|| few.status_at(Some(SEAT), None, None).unwrap());
    let (many_full, full) = work(|| many.status_at(Some(SEAT), None, None).unwrap());
    // The full reduction grows with the history; this is what the summary avoids.
    assert!(many_full.vm_steps > few_full.vm_steps * 10, "50={few_full:?}, 3000={many_full:?}");
    assert!(
        many_summary.vm_steps <= few_summary.vm_steps + 2_000,
        "the summary read the seat's history: 50={few_summary:?}, 3000={many_summary:?}"
    );
    // It answers what the one-field callers read, as the full reduction does.
    let (summary, full) = (&summary.subjects[0], &full.subjects[0]);
    assert!(summary.claims.is_empty());
    assert_eq!(full.claims.len(), 3_000);
    let (summary_actual, full_actual) = (summary.actual.as_ref().unwrap(), full.actual.as_ref().unwrap());
    for field in ["incarnation_id", "status"] {
        assert_eq!(summary_actual[field], full_actual[field], "{field}");
    }
    assert_eq!(summary_actual["incarnation_id"], "incarnation-2999");
    assert_eq!(summary.desired, full.desired);
    assert!(summary.harness.is_none());
    assert_eq!(summary.projection.layer, full.projection.layer);
}

#[test]
fn a_summary_takes_each_field_from_the_newest_observation_that_has_it() {
    let store = seat_with(10);
    observe(&store, json!({"status": "stopped"}));
    let summary = store.status_summary_at(SEAT, None).unwrap();
    let full = store.status_at(Some(SEAT), None, None).unwrap();
    let (summary, full) = (summary.subjects[0].actual.as_ref().unwrap(), full.subjects[0].actual.as_ref().unwrap());
    assert_eq!(summary["status"], "stopped");
    assert_eq!(summary["incarnation_id"], "incarnation-9");
    for field in ["incarnation_id", "status"] {
        assert_eq!(summary[field], full[field], "{field}");
    }
}

/// A small deterministic generator, so each seed's history can be replayed.
struct Seeded(u64);

impl Seeded {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound
    }
}

/// One seat's history, written as rows the way any member or binary could leave them: claims
/// from three origins, accepted out of order and with ties, batches of one or several claims,
/// restarts, stops, explicit nulls, repaired claims, and kinds the actual state leaves out
/// carrying the same fields.
fn random_history(seed: u64) -> Store {
    let store = Store::open_memory("node").unwrap();
    let intent = crate::graph::parse_test_intent(
        "version 2\nagent \"example/worker\" { command \"true\"; workspace \"/tmp\"; restart \"never\"; }\n",
        store.origin(),
    )
    .unwrap();
    let expected = intent
        .subjects
        .keys()
        .map(|subject| (subject.clone(), store.selected_desired_token(subject).unwrap().into_iter().collect()))
        .collect();
    store.apply(&intent, &expected, "history-declare").unwrap();
    let mut random = Seeded(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let connection = store.connection.write();
    if random.below(3) == 0 {
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h4b8a1cb72b7d owner=fractal_store::store::status_summary_tests TODO=bound-or-reference-values-and-reclamation */ "UPDATE desired SET owner_run='mission-run/example-owner', owner_generation='run-generation/example-owner', owner_step='step-run/example-owner/work' WHERE subject=?1",
                [SEAT],
            )
            .unwrap();
    }
    let origins = ["node", "peer-a", "peer-b"];
    let mut sequences = [100_u64; 3];
    let mut incarnation = 0;
    let mut batch: Option<(String, String)> = None;
    let mut accepted = 1_700_000_000_000_u64;
    for index in 0..(10 + random.below(50)) {
        // Mostly forward, sometimes the same millisecond, sometimes an earlier claim arriving late.
        accepted = match random.below(6) {
            0 => accepted,
            1 => accepted - random.below(5_000).min(accepted - 1_600_000_000_000),
            _ => accepted + 1 + random.below(2_000),
        };
        let (kind, fields) = match random.below(12) {
            0..=2 => {
                incarnation += 1;
                ("runtime.observed", json!({"status": "running", "incarnation_id": format!("incarnation-{incarnation}"), "runtime_id": SEAT}))
            }
            3 | 4 => {
                let status = ["running", "idle", "waiting"][random.below(3) as usize];
                ("runtime.observed", json!({"status": status}))
            }
            5 => ("runtime.observed", json!({"status": "stopped"})),
            6 => ("runtime.observed", json!({"status": "exited", "incarnation_id": null})),
            7 => ("workspace.observed", json!({"status": "dirty", "incarnation_id": format!("workspace-{index}")})),
            8 => (["harness.turn", "Harness.turn"][random.below(2) as usize], json!({"status": "harness", "incarnation_id": format!("harness-{index}")})),
            9 => ("reconcile.fault", json!({"status": "fault", "incarnation_id": format!("fault-{index}")})),
            10 => ("runtime.readiness-deadline-reached", json!({"status": "late", "incarnation_id": format!("late-{index}")})),
            _ => ("runtime.observed", json!({"incarnation_id": format!("incarnation-{incarnation}")})),
        };
        // A new batch, or the claim joins the previous one.
        if batch.is_none() || random.below(4) != 0 {
            let which = random.below(3) as usize;
            sequences[which] += 1;
            let id = format!("history-batch-{index}");
            connection
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h7da5f6391087 owner=fractal_store::store::status_summary_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES (?1,?2,?3,'fixture',?4)",
                    rusqlite::params![id, origins[which], sequences[which], accepted.to_string()],
                )
                .unwrap();
            batch = Some((id, origins[which].to_owned()));
        }
        let (batch_id, origin) = batch.clone().unwrap();
        let id = format!("history-claim-{index:03}-{}", random.below(1_000));
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h98e5236bc06a owner=fractal_store::store::status_summary_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms) VALUES (?1,?2,?3,?4,?5,?3,?6,'[]',?7)",
                rusqlite::params![id, batch_id, SEAT, kind, origin, json!({"fields": fields}).to_string(), accepted.to_string()],
            )
            .unwrap();
        if random.below(8) == 0 {
            connection.execute(/* sqlite-large-value: allow-for-now lv-h6fb9eeec970c owner=fractal_store::store::status_summary_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO projection_digest_repaired_claims(id) VALUES (?1)", [&id]).unwrap();
        }
    }
    drop(connection);
    store
}

/// Every seed whose summary answers a different status or incarnation than the full reduction,
/// read now and at a few earlier cuts.
fn summary_differences(seeds: std::ops::Range<u64>) -> Vec<String> {
    let mut differences = Vec::new();
    for seed in seeds {
        let store = random_history(seed);
        let newest = store.readers.get().query_row("SELECT MAX(store_index) FROM claims", [], |row| row.get::<_, u64>(0)).unwrap();
        for cut in [None, Some(newest - 3), Some(newest - 9)] {
            let summary = store.status_summary_at(SEAT, cut).unwrap().subjects.remove(0);
            let full = store.status_at(Some(SEAT), None, cut).unwrap().subjects.remove(0);
            let pick = |status: &SubjectStatus| {
                let actual = status.actual.clone().unwrap_or_default();
                (actual["status"].clone(), actual["incarnation_id"].clone())
            };
            if pick(&summary) != pick(&full) {
                differences.push(format!("seed {seed} cut {cut:?}: summary {:?}, full {:?}", pick(&summary), pick(&full)));
            }
        }
    }
    differences
}

#[test]
fn a_summary_answers_the_full_reductions_status_and_incarnation_over_random_histories() {
    let differences = summary_differences(0..64);
    assert!(differences.is_empty(), "{differences:#?}");
}

/// Each term of the actual fold's filter is needed for the summary to agree with the full fold,
/// and the random histories reach every one: a field read that leaves one out disagrees.
#[test]
fn a_field_read_whose_filter_differs_from_the_full_folds_is_caught() {
    let terms = [
        ("repaired claims", format!("{ACTUAL_STATE_CLAIM} AND claims.kind NOT LIKE 'harness.%'")),
        ("harness kinds in any case", format!("{ACTUAL_STATE_CLAIM} AND {UNREPAIRED_CLAIM}")),
        (
            "declarations, deadlines and faults",
            format!("(claims.kind<'harness.' OR claims.kind>='harness/') AND claims.kind NOT LIKE 'harness.%' AND {UNREPAIRED_CLAIM}"),
        ),
    ];
    for (term, filter) in terms {
        ACTUAL_FIELDS_FILTER_FOR_TEST.with(|slot| *slot.borrow_mut() = Some(filter));
        let differences = summary_differences(0..64);
        ACTUAL_FIELDS_FILTER_FOR_TEST.with(|slot| *slot.borrow_mut() = None);
        assert!(!differences.is_empty(), "a field read that keeps {term} still agreed on every history");
    }
    // The override itself, given the same filter, agrees.
    ACTUAL_FIELDS_FILTER_FOR_TEST.with(|slot| *slot.borrow_mut() = Some(ACTUAL_FOLD_CLAIM.clone()));
    let differences = summary_differences(0..64);
    ACTUAL_FIELDS_FILTER_FOR_TEST.with(|slot| *slot.borrow_mut() = None);
    assert!(differences.is_empty(), "{differences:#?}");
}

/// Times a seat's full and incarnation reductions on a store copy, read-only and unchanged:
/// `ST_STATUS_SUMMARY_BENCH_STORE` names the file and `ST_STATUS_SUMMARY_BENCH_SEATS` the seats,
/// comma-separated. Each seat's incarnation reduction runs first, on a cold cache.
#[test]
#[ignore = "requires ST_STATUS_SUMMARY_BENCH_STORE, a store copy, and ST_STATUS_SUMMARY_BENCH_SEATS"]
fn status_summary_benchmark() {
    let path = std::env::var("ST_STATUS_SUMMARY_BENCH_STORE").unwrap();
    let connection = Connection::open_with_flags(
        format!("file:{path}?mode=ro&immutable=1"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();
    for seat in std::env::var("ST_STATUS_SUMMARY_BENCH_SEATS").unwrap().split(',') {
        for claims in [ClaimsReduction::Incarnation, ClaimsReduction::Full, ClaimsReduction::Incarnation, ClaimsReduction::Full] {
            let started = std::time::Instant::now();
            let (status, _) = subject_status_at(&connection, seat, None, None, claims).unwrap().unwrap();
            let actual = status.actual.unwrap_or_default();
            eprintln!(
                "{seat} {claims:?}: {:.1} ms, {} claim ids, incarnation {}, status {}",
                started.elapsed().as_secs_f64() * 1000.0, status.claims.len(), actual["incarnation_id"], actual["status"]
            );
        }
    }
}


