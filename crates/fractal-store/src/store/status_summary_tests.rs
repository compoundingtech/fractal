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
fn owned_set_member_fields_avoid_the_full_claim_vector_at_current_and_older_cuts() {
    let store = seat_with(3_000);
    let cut = store.index().unwrap();
    observe(&store, json!({
        "status": "running", "incarnation_id": "after-pinned-cut", "runtime_id": "after-pinned-cut",
    }));
    assert!(store.index().unwrap() > cut, "fixture must retain a genuinely older cut");
    for at in [None, Some(cut)] {
        let (summary_work, summary) = work(|| store.generic_status_reduction(
            Some(SEAT), None, at, false, ClaimsReduction::Fields,
        ).unwrap());
        let (full_work, full) = work(|| store.generic_status_reduction(
            Some(SEAT), None, at, false, ClaimsReduction::Full,
        ).unwrap());
        assert!(summary.subjects[0].claims.is_empty(), "hot path enumerated claim ids");
        assert_eq!(summary.subjects[0].claims_summary.count, 0);
        assert_eq!(full.subjects[0].claims.len(), if at.is_some() { 3_000 } else { 3_001 });
        assert_eq!(full.subjects[0].actual.as_ref().unwrap()["incarnation_id"],
            if at.is_some() { "incarnation-2999" } else { "after-pinned-cut" });
        let actual_scope = smallclaims::sqlite::work::SqliteWorkScope::start();
        let actual = store.actual_for_generic_reads(SEAT, at).unwrap();
        let actual_work = actual_scope.finish();
        assert_eq!(actual, full.subjects[0].actual);
        assert!(actual_work.vm_steps < full_work.vm_steps,
            "actual={actual_work:?}, full={full_work:?}");
        assert!(summary_work.vm_steps < full_work.vm_steps,
            "summary={summary_work:?}, full={full_work:?}");
        let mut summary = serde_json::to_value(summary).unwrap();
        let mut full = serde_json::to_value(full).unwrap();
        for response in [&mut summary, &mut full] {
            response["subjects"][0].as_object_mut().unwrap().remove("claims");
            response["subjects"][0].as_object_mut().unwrap().remove("claims_summary");
        }
        assert_eq!(summary, full, "only claim provenance representation may change");
    }
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

#[test]
fn notice_summary_does_not_advance_a_populated_shared_cache_over_a_claim_backlog() {
    let store = seat_with(2);
    let warm_cut = store.index().unwrap();
    store.status_at(Some(SEAT), None, Some(warm_cut)).unwrap();
    let before = {
        let cache = store.fractal.subject_cache.lock().unwrap();
        assert!(!cache.statuses.is_empty(), "exercise an already populated cache");
        (cache.through, cache.statuses.len())
    };
    for index in 0..32 {
        notice_raw_observation(
            &store, "agent/example/backlog", "runtime.observed",
            &format!("summary-backlog-{index}"), "backlog-origin", 100 + index,
            json!({"status":"running", "incarnation_id":"unrelated"}),
        );
    }
    let through = store.index().unwrap();
    assert!(through > before.0, "the cache has a claim backlog to process");
    for cut in [None, Some(through)] {
        let summary = store.status_summary_at(SEAT, cut).unwrap();
        assert_eq!(summary.store_index, through);
        let cache = store.fractal.subject_cache.lock().unwrap();
        assert_eq!((cache.through, cache.statuses.len()), before,
            "an incarnation-only reduction must neither advance nor maintain the shared cache");
    }
}


/// Model retained or replicated rows without the public append boundary rejecting reserved kinds.
fn notice_raw_observation(
    store: &Store,
    subject: &str,
    kind: &str,
    id: &str,
    origin: &str,
    sequence: u64,
    fields: serde_json::Value,
) -> u64 {
    let connection = store.connection.write();
    let accepted = (1_700_000_000_000 + sequence).to_string();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h7da5f6391087 owner=fractal_store::store::status_summary_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES (?1,?2,?3,'fixture',?4)",
            rusqlite::params![id, origin, sequence, accepted],
        )
        .unwrap();
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h87bbf137f13b owner=fractal_store::store::status_summary_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms) VALUES (?1,?1,?2,?3,?4,?5,'[]',?6)",
            rusqlite::params![id, subject, kind, origin, json!({"fields": fields}).to_string(), accepted],
        )
        .unwrap();
    u64::try_from(connection.last_insert_rowid()).unwrap()
}

#[test]
fn notice_incarnation_reduction_excludes_reserved_fields_and_source_identity_without_generic_guard() {
    for (kind, safe_kind) in [("attention.notice", "runtime.observed"),
        ("custom.attention.snapshot", "custom.health-observed")] {
        let store = Store::open_memory("node").unwrap();
        notice_raw_observation(
            &store, SEAT, safe_kind, "safe-actual", "safe-origin", 1,
            json!({"status": "running", "incarnation_id": "safe-incarnation"}),
        );
        notice_raw_observation(
            &store, SEAT, kind, "reserved-later", "reserved-origin", 2,
            json!({"status": "reserved-status", "incarnation_id": "reserved-incarnation"}),
        );
        const RESERVED_ONLY: &str = "agent/example/reserved-only";
        notice_raw_observation(
            &store, RESERVED_ONLY, kind, "reserved-only", "reserved-origin", 3,
            json!({"status": "reserved-only-status", "incarnation_id": "reserved-only-incarnation"}),
        );
        // Deliberately bypass Store::status_summary_at's generic visibility guard: the
        // incarnation fold and its source attribution must each exclude reserved kinds.
        let connection = store.readers.get();
        let (safe, _) = subject_status_at(&connection, SEAT, None, None, ClaimsReduction::Incarnation)
            .unwrap().unwrap();
        let actual = safe.actual.as_ref().unwrap();
        assert_eq!(actual["status"], "running", "{kind}: {safe:?}");
        assert_eq!(actual["incarnation_id"], "safe-incarnation", "{kind}: {safe:?}");
        assert_eq!(safe.actual_claim.as_deref(), Some("safe-actual"), "{kind}: {safe:?}");
        assert_eq!(safe.actual_origin.as_deref(), Some("safe-origin"), "{kind}: {safe:?}");

        if let Some((reserved, _)) =
            subject_status_at(&connection, RESERVED_ONLY, None, None, ClaimsReduction::Incarnation).unwrap()
        {
            assert!(reserved.actual.is_none(), "{kind}: {reserved:?}");
            assert!(reserved.actual_claim.is_none(), "{kind}: {reserved:?}");
            assert!(reserved.actual_origin.is_none(), "{kind}: {reserved:?}");
        }
    }
}

#[test]
fn notice_status_summary_refuses_contamination_but_serves_the_safe_historical_cut() {
    for kind in ["attention.notice", "custom.attention.snapshot"] {
        let store = Store::open_memory("node").unwrap();
        let safe_cut = notice_raw_observation(
            &store, SEAT, "runtime.observed", "safe-actual", "safe-origin", 1,
            json!({"status": "running", "incarnation_id": "safe-incarnation"}),
        );
        let contaminated_cut = notice_raw_observation(
            &store, SEAT, kind, "reserved-later", "reserved-origin", 2,
            json!({"status": "reserved-status", "incarnation_id": "reserved-incarnation"}),
        );
        for cut in [None, Some(contaminated_cut)] {
            let error = store.status_summary_at(SEAT, cut).unwrap_err();
            assert_eq!(
                error.downcast_ref::<FractalError>().expect("explicit notice refusal").code,
                "notices-unavailable",
                "{kind} at {cut:?}: {error:?}",
            );
        }
        let safe = store.status_summary_at(SEAT, Some(safe_cut)).unwrap().subjects.remove(0);
        let actual = safe.actual.as_ref().unwrap();
        assert_eq!(actual["status"], "running", "{kind}: {safe:?}");
        assert_eq!(actual["incarnation_id"], "safe-incarnation", "{kind}: {safe:?}");
        assert_eq!(safe.actual_claim.as_deref(), Some("safe-actual"), "{kind}: {safe:?}");
        assert_eq!(safe.actual_origin.as_deref(), Some("safe-origin"), "{kind}: {safe:?}");
    }
}

#[test]
fn notice_status_summary_pins_visibility_fields_and_source_before_a_concurrent_reserved_append() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("summary.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    notice_raw_observation(
        &store, SEAT, "runtime.observed", "safe-actual", "safe-origin", 1,
        json!({"status": "running", "incarnation_id": "safe-incarnation"}),
    );
    let writer = Store::open(&path, "node").unwrap();
    STATUS_AFTER_DESIRED_READ.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            std::thread::spawn(move || {
                notice_raw_observation(
                    &writer, SEAT, "custom.attention.snapshot", "reserved-later", "reserved-origin", 2,
                    json!({"status": "reserved-status", "incarnation_id": "reserved-incarnation"}),
                );
                // A safe observation also lands after the request's cut. Reserved-kind
                // filtering alone cannot prevent a mixed snapshot from selecting this row.
                notice_raw_observation(
                    &writer, SEAT, "runtime.observed", "safe-after-cut", "safe-origin", 3,
                    json!({"status": "stopped", "incarnation_id": "later-incarnation"}),
                );
            })
            .join()
            .unwrap();
        }));
    });
    let safe = store.status_summary_at(SEAT, None).unwrap().subjects.remove(0);
    assert!(
        STATUS_AFTER_DESIRED_READ.with(|slot| slot.borrow().is_none()),
        "the interleaving hook must run during the summary reduction",
    );
    let actual = safe.actual.as_ref().unwrap();
    assert_eq!(actual["status"], "running", "{safe:?}");
    assert_eq!(actual["incarnation_id"], "safe-incarnation", "{safe:?}");
    assert_eq!(safe.actual_claim.as_deref(), Some("safe-actual"), "{safe:?}");
    assert_eq!(safe.actual_origin.as_deref(), Some("safe-origin"), "{safe:?}");
    let error = store.status_summary_at(SEAT, None).unwrap_err();
    assert_eq!(
        error.downcast_ref::<FractalError>().expect("explicit notice refusal").code,
        "notices-unavailable",
        "{error:?}",
    );
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
                    /* sqlite-large-value: allow-for-now lv-hb4ac3d3059e1 owner=fractal_store::store::status_summary_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES (?1,?2,?3,'fixture',?4)",
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

/// The notice check and the unknown-claim check read a subject's kinds with one seek per kind.
/// At every cut, over random subjects whose kinds share prefixes and differ only in case or in
/// a trailing character, that must be exactly the kinds a DISTINCT over every claim finds.
#[test]
fn a_subjects_kinds_by_seeks_equal_a_distinct_over_its_claims_at_every_cut() {
    const KINDS: &[&str] = &[
        "a", "a.b", "a.b.c", "ab", "a-b", "a/b", "A", "harness.turn", "harness/", "Harness.turn",
        "attention.notice", "attention.notice-x", "attention.noticex", "custom.attention",
        "custom.attention.x", "custom.attentionx", "runtime.observed", "runtime.observed2", "z", "é",
    ];
    let store = Store::open_memory("node").unwrap();
    let connection = store.connection.write();
    connection
        .execute(/* sqlite-large-value: allow-for-now lv-h06cff359fe46 owner=fractal_store::store::status_summary_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms) VALUES ('kinds','node',9000,'fixture','1700000000000')", [])
        .unwrap();
    let subjects = ["agent/example/kinds-0", "agent/example/kinds-1", "agent/example/kinds-1x", "agent/example/kinds-2"];
    let mut random = Seeded(0x5EED_C1A1_u64);
    for index in 0..1_500 {
        // Kinds arrive in a random order, often repeated, so each kind's first claim is at a
        // different cut on each subject.
        let subject = subjects[random.below(subjects.len() as u64) as usize];
        let kind = KINDS[random.below(KINDS.len() as u64) as usize];
        connection
            .execute(
                /* sqlite-large-value: allow-for-now lv-h6dbcaf0dfacb owner=fractal_store::store::status_summary_tests TODO=prepare-body-and-predecessor-values */ "INSERT INTO claims(id,batch_id,subject,kind,origin,body,predecessors,accepted_at_unix_ms) VALUES (?1,'kinds',?2,?3,'node','{}','[]','1700000000000')",
                rusqlite::params![format!("kinds-{index}"), subject, kind],
            )
            .unwrap();
    }
    let newest: u64 = connection.query_row("SELECT MAX(store_index) FROM claims", [], |row| row.get(0)).unwrap();
    let first: u64 = connection.query_row("SELECT MIN(store_index) FROM claims WHERE id LIKE 'kinds-%'", [], |row| row.get(0)).unwrap();
    let mut distinct = connection
        .prepare("SELECT DISTINCT kind FROM claims WHERE subject=?1 AND store_index<=?2 ORDER BY kind")
        .unwrap();
    let mut cuts_that_hide_a_kind = 0;
    for subject in subjects.iter().chain(&["agent/example/kinds", "agent/example/none"]) {
        for cut in (first.saturating_sub(1)..=newest).chain([u64::MAX >> 1]) {
            let expected = distinct
                .query_map(rusqlite::params![subject, cut], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let sought = subject_kinds_at(&connection, subject, cut).unwrap();
            assert_eq!(sought, expected, "{subject} at {cut}");
            if expected.len() < subject_kinds_at(&connection, subject, newest).unwrap().len() {
                cuts_that_hide_a_kind += 1;
            }
        }
    }
    // The cuts reach before kinds a subject gains later, so a read that ignored the cut fails.
    assert!(cuts_that_hide_a_kind > 100, "{cuts_that_hide_a_kind}");
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


/// Generic visibility must cost subject/kind probes, not runtime history scans.
#[test]
fn generic_visibility_probes_reserved_kinds_without_reading_runtime_history() {
    const BEFORE: &str =
        "SELECT DISTINCT claims.subject,claims.kind FROM json_each(?1) selected
         JOIN claims ON claims.subject=selected.value WHERE claims.store_index<=?2";
    let store = seat_with(3_000);
    let names = BTreeSet::from([SEAT.to_owned()]);
    let cut = store.index().unwrap();
    let encoded = serde_json::to_string(&names).unwrap();
    let connection = store.readers.get();
    for (label, sql) in [("before", BEFORE), ("after", Store::GENERIC_RESERVED_SUBJECTS_SQL)] {
        let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap()
            .query_map(params![encoded, cut], |row| row.get::<_, String>(3)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        eprintln!("generic visibility {label} plan: {plan:?}");
        if label == "after" {
            assert!(!plan.iter().any(|line| line.contains("TEMP B-TREE")), "{plan:?}");
            // Either existing subject/kind covering index can win a prefix probe.
            assert_eq!(plan.iter().filter(|line| line.contains(
                "COVERING INDEX claims_subject_kind")).count(), 4, "{plan:?}");
        }
    }
    let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let started = std::time::Instant::now();
    let before = connection.prepare(BEFORE).unwrap()
        .query_map(params![encoded, cut], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let before_elapsed = started.elapsed();
    let before_work = scope.finish();
    assert_eq!(before, vec![(SEAT.to_owned(), "runtime.observed".into())]);
    drop(connection);
    let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let started = std::time::Instant::now();
    assert_eq!(store.generic_status_subject_names_at(names, cut).unwrap(),
        BTreeSet::from([SEAT.to_owned()]));
    let after_elapsed = started.elapsed();
    let after_work = scope.finish();
    eprintln!("generic visibility 3000 observations: before={before_work:?} {before_elapsed:?}; after={after_work:?} {after_elapsed:?}");
    assert!(before_work.vm_steps > after_work.vm_steps * 10,
        "before={before_work:?}, after={after_work:?}");
}

#[test]
fn generic_visibility_reserved_kind_probes_match_namespace_and_snapshot_boundaries() {
    let kinds = [
        "custom.attention", "custom.attention.", "custom.attention.source-created",
        "attention.notice", "attention.notice-", "attention.notice-source",
        "custom.attention-", "custom.attention/", "custom.attentionX",
        "attention.notice.", "attention.notice/", "attention.noticeX", "runtime.observed",
    ];
    let store = seat_with(kinds.len());
    let names = kinds.iter().enumerate().map(|(index, _)| format!("agent/kind-{index}"))
        .collect::<BTreeSet<_>>();
    // Rewrite only this in-memory fixture to exercise hostile and legacy kinds
    // independently of today's write-time schema validation.
    for (index, kind) in kinds.iter().enumerate() {
        store.connection.write().execute(
            "UPDATE claims SET subject=?1,kind=?2 WHERE store_index=?3",
            params![format!("agent/kind-{index}"), kind, index + 1],
        ).unwrap();
    }
    for cut in 0..=kinds.len() as u64 {
        let expected = kinds.iter().enumerate().filter_map(|(index, kind)| {
            (index as u64 >= cut || !fractal_schema::notice::is_reserved_kind(kind))
                .then(|| format!("agent/kind-{index}"))
        }).collect::<BTreeSet<_>>();
        assert_eq!(store.generic_status_subject_names_at(names.clone(), cut).unwrap(),
            expected, "cut={cut}");
    }
    let roots = BTreeSet::from([
        "custom/attention".to_owned(), "custom/attention/source/x".to_owned(),
    ]);
    assert!(store.generic_status_subject_names_at(roots, 0).unwrap().is_empty());
}
