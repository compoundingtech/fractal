use super::*;

const START: u64 = 10 * DAY;
fn target() -> Identity {
    Identity { dotfiles_revision: "1111111111111111111111111111111111111111".into(),
        factory_revision: "2222222222222222222222222222222222222222".into(),
        daemon_revision: "3333333333333333333333333333333333333333".into(),
        system_path: None, home_path: None }
}
fn declaration(id: &str, hosts: &[&str]) -> Receipt {
    Receipt::RollDeclared { version: 1, roll_id: id.into(), declared_at_ms: START,
        targets: hosts.iter().map(|host| ((*host).into(), target())).collect() }
}
fn observation(id: &str, host: &str, at: u64, adoption: Adoption) -> Receipt {
    Receipt::HostObserved { version: 1, roll_id: id.into(), declared_at_ms: START,
        host: host.into(), observed_at_ms: at, target: target(), adoption,
        evidence: (adoption != Adoption::Unknown).then(|| Verification {
            system_path: "/nix/store/example-system".into(), home_path: "/nix/store/example-home".into(),
            expected_system_path: "/nix/store/example-system".into(), expected_home_path: "/nix/store/example-home".into(),
            daemon_ready: adoption == Adoption::Verified, daemon_revision: target().daemon_revision, factory_revision: target().factory_revision }) }
}
fn sample(registry: &Registry, id: &str, now: u64) -> Option<(u64, bool)> {
    outcome(registry.rolls.get(id).unwrap(), now)
}

#[test]
fn cohort_and_observations_are_immutable_and_retries_do_not_restart_clock() {
    let mut registry = Registry::default();
    assert!(registry.accept(declaration("roll", &["host-a", "host-b"]), START).unwrap());
    assert!(!registry.accept(declaration("roll", &["host-a", "host-b"]), START + DAY).unwrap());
    assert!(registry.accept(declaration("roll", &["host-a"]), START + DAY).is_err());
    let mut retimed = declaration("roll", &["host-a", "host-b"]);
    if let Receipt::RollDeclared { declared_at_ms, .. } = &mut retimed { *declared_at_ms += 1; }
    assert!(registry.accept(retimed, START + DAY).is_err());
    let receipt = observation("roll", "host-a", START + 10, Adoption::Verified);
    assert!(registry.accept(receipt.clone(), START + 10).unwrap());
    assert!(!registry.accept(receipt, START + DAY).unwrap());
    assert!(registry.accept(observation("roll", "host-a", START + 10, Adoption::Unknown), START + DAY).is_err());
    assert!(registry.accept(observation("roll", "host-c", START + 10, Adoption::Verified), START + DAY).is_err());
}

#[test]
fn exact_independent_evidence_is_required_not_just_adoption_label() {
    let mut registry = Registry::default();
    registry.accept(declaration("roll", &["host-a"]), START).unwrap();
    for field in ["daemon", "factory", "ready", "system", "home", "expected", "absent", "target"] {
        let mut receipt = observation("roll", "host-a", START + 1, Adoption::Verified);
        if let Receipt::HostObserved { evidence, target, .. } = &mut receipt {
            let proof = evidence.as_mut().unwrap();
            match field {
                "daemon" => proof.daemon_revision.truncate(7),
                "factory" => proof.factory_revision = "old-factory".into(),
                "ready" => proof.daemon_ready = false,
                "system" => proof.system_path = "old-system".into(),
                "home" => proof.home_path = "old-home".into(),
                "expected" => { proof.system_path.clear(); proof.expected_system_path.clear(); },
                "absent" => *evidence = None,
                "target" => target.dotfiles_revision = "other".into(),
                _ => unreachable!(),
            }
        }
        assert!(registry.accept(receipt, START + 1).is_err(), "{field}");
    }
    registry.accept(observation("roll", "host-a", START + 1, Adoption::Verified), START + 1).unwrap();
    assert_eq!(sample(&registry, "roll", START + 2), Some((START + 1, false)));
}

#[test]
fn unknown_and_predeadline_nonadoption_never_prove_a_miss_even_after_holds() {
    let mut registry = Registry::default();
    registry.accept(declaration("roll", &["host-a", "held-host"]), START).unwrap();
    registry.accept(observation("roll", "host-a", START + 10, Adoption::Verified), START + 10).unwrap();
    registry.accept(observation("roll", "held-host", START + 20, Adoption::NotAdopted), START + 20).unwrap();
    registry.accept(observation("roll", "held-host", START + 2 * DAY, Adoption::Unknown), START + 2 * DAY).unwrap();
    assert_eq!(sample(&registry, "roll", START + 3 * DAY), None);
    registry.accept(observation("roll", "held-host", START + 4 * DAY, Adoption::NotAdopted), START + 4 * DAY).unwrap();
    assert_eq!(sample(&registry, "roll", START + 4 * DAY), Some((START + DAY, true)));
    registry.accept(observation("roll", "held-host", START + 5 * DAY, Adoption::Verified), START + 5 * DAY).unwrap();
    assert_eq!(sample(&registry, "roll", START + 5 * DAY), Some((START + DAY, true)));
}

#[test]
fn final_host_sets_completion_and_late_verification_cannot_backdate_installation() {
    let mut registry = Registry::default();
    registry.accept(declaration("timely", &["host-a", "host-b"]), START).unwrap();
    registry.accept(observation("timely", "host-a", START + 1, Adoption::Verified), START + 1).unwrap();
    registry.accept(observation("timely", "host-b", START + DAY, Adoption::Verified), START + DAY).unwrap();
    assert_eq!(sample(&registry, "timely", START + DAY), Some((START + DAY, false)));
    registry.accept(observation("timely", "host-b", START + 2 * DAY, Adoption::Unknown), START + 2 * DAY).unwrap();
    assert_eq!(sample(&registry, "timely", START + 2 * DAY), Some((START + DAY, false)));
    registry.accept(declaration("late", &["host-a"]), START).unwrap();
    registry.accept(observation("late", "host-a", START + DAY + 1, Adoption::Verified), START + DAY + 1).unwrap();
    assert_eq!(sample(&registry, "late", START + DAY), None);
    assert_eq!(sample(&registry, "late", START + DAY + 1), Some((START + DAY, true)));
    let aged = render(&registry, true, START + 32 * DAY);
    assert_eq!(aged["measurement"]["windows"][0]["eligible"], 0.0);
    assert_eq!(aged["status"], "no-data");
}

#[test]
fn known_results_and_roll_denominator_survive_independent_unknown_rolls() {
    let mut registry = Registry::default();
    registry.accept(declaration("completed", &["host-a", "host-b"]), START).unwrap();
    for host in ["host-a", "host-b"] {
        registry.accept(observation("completed", host, START + 100, Adoption::Verified), START + 100).unwrap();
    }
    registry.accept(declaration("unknown", &["host-c"]), START + 100).unwrap();
    let row = render(&registry, true, START + 101);
    assert_eq!(row["measurement"]["windows"][0]["eligible"], 1.0);
    assert_eq!(row["measurement"]["unknown_open_rolls"], 1);
    assert_eq!(row["remaining_percent"], 100.0);
    assert_eq!(row["measurement"]["external_population_coverage"], "unknown");
    assert_eq!(row["measurement"]["population_frontier_ms"], START + 101);
    assert!(render(&registry, false, START + 101)["measurement"]["population_frontier_ms"].is_null());
}

#[test]
fn durable_restart_replays_receipts_and_preserves_idempotency_and_historical_accounting() {
    let directory = tempfile::tempdir().unwrap();
    let now = START + 2 * DAY;
    {
        let store = Store::open_memory("node").unwrap();
        install(&store, directory.path()).unwrap();
        ingest(&store, declaration("roll", &["host-a"]), START).unwrap();
        ingest(&store, observation("roll", "host-a", now, Adoption::Verified), now).unwrap();
        assert_eq!(row(&store, now)["status"], "exhausted");
    }
    let store = Store::open_memory("node").unwrap();
    install(&store, directory.path()).unwrap();
    assert_eq!(ingest(&store, observation("roll", "host-a", now, Adoption::Verified), now + 1).unwrap()["changed"], false);
    assert_eq!(row(&store, now)["measurement"]["windows"][0]["bad"], 1.0);
    assert_eq!(row(&store, START + DAY)["measurement"]["windows"][0]["eligible"], 0.0);
    assert!(ingest(&store, declaration("roll", &["host-b"]), now + 1).is_err());
    // A pinned graph read observes only the projection, not source database work.
    store.read_snapshot(|_| { assert_eq!(row(&store, now)["status"], "exhausted"); Ok(()) }).unwrap();
}

#[test]
fn future_unknown_versions_empty_targets_and_undeclared_observations_are_rejected() {
    let mut registry = Registry::default();
    assert!(registry.accept(declaration("empty", &[]), START).is_err());
    assert!(registry.accept(observation("missing", "host-a", START + 1, Adoption::Unknown), START + 1).is_err());
    let mut receipt = declaration("future", &["host-a"]);
    if let Receipt::RollDeclared { version, .. } = &mut receipt { *version = 2; }
    assert!(registry.accept(receipt, START).is_err());
    assert!(registry.accept(declaration("future", &["host-a"]), START - 1).is_err());
}

#[test]
fn slow_burn_requires_both_windows_independently_of_horizon_exhaustion() {
    let mut registry = Registry::default();
    for index in 0..100 {
        let id = format!("timely-{index}");
        registry.accept(declaration(&id, &["host-a"]), START).unwrap();
        registry.accept(observation(&id, "host-a", START + 1, Adoption::Verified), START + 1).unwrap();
    }
    registry.accept(declaration("late", &["host-a"]), START).unwrap();
    registry.accept(observation("late", "host-a", START + DAY + 1, Adoption::Verified), START + DAY + 1).unwrap();
    assert_eq!(render(&registry, true, START + DAY + 60_000)["status"], "burning");
    // The 30m short window loses the bad event while the 6h long window retains it.
    assert_eq!(render(&registry, true, START + DAY + 1_800_001)["status"], "not-eligible");
}

#[tokio::test]
async fn api_ingestion_publishes_immediately_and_signals_native_readers() {
    use axum::{Json, extract::State};
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::tests_shared::state(root.path());
    install(&state.store, root.path()).unwrap();
    let mut changes = state.event_notify.subscribe();
    let accepted = crate::api::operations::ingest_roll(
        State(state.clone()), Json(declaration("roll", &["host-a"]))).await.unwrap().0;
    assert_eq!(accepted["changed"], true);
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    let rows = crate::api::operations::current_slo_rows(&state.store, root.path()).unwrap();
    let row = rows.iter().find(|row| row["id"] == OBJECTIVE).unwrap();
    assert_eq!(row["measurement"]["accepted_declarations"], 1);
    assert_eq!(row["measurement"]["unknown_open_rolls"], 1);
    let error = crate::api::operations::ingest_roll(
        State(state), Json(declaration("roll", &["host-b"]))).await.unwrap_err();
    assert_eq!(error.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[test]
fn readers_do_not_acquire_the_durable_writer_lock() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("node").unwrap();
    install(&store, root.path()).unwrap();
    let source = store.fractal.extensions.get::<Source>();
    let _writer = source.writer.lock().unwrap();
    assert_eq!(row(&store, START)["status"], "no-data");
}

#[test]
fn overdue_unknown_gates_coverage_without_erasing_known_good_or_bad_outcomes() {
    let mut registry = Registry::default();
    registry.accept(declaration("completed", &["host-a"]), START).unwrap();
    registry.accept(observation("completed", "host-a", START + DAY - 100, Adoption::Verified), START + DAY).unwrap();
    registry.accept(declaration("unknown", &["host-b"]), START).unwrap();
    let row = render(&registry, true, START + DAY + 10);
    assert_eq!(row["measurement"]["windows"][0]["eligible"], 1.0);
    assert_eq!(row["measurement"]["windows"][0]["unknown"], 1);
    assert_eq!(row["measurement"]["windows"][0]["coverage"], 0.5);
    assert_eq!(row["status"], "no-data");
    assert!(row["remaining_percent"].is_null());
    registry.accept(declaration("late", &["host-c"]), START).unwrap();
    registry.accept(observation("late", "host-c", START + DAY + 1, Adoption::NotAdopted), START + DAY + 1).unwrap();
    let row = render(&registry, true, START + DAY + 10);
    assert_eq!(row["measurement"]["windows"][0]["bad"], 1.0);
    assert_eq!(row["measurement"]["windows"][0]["eligible"], 2.0);
    assert_eq!(row["status"], "no-data");
}

#[test]
fn missing_partial_or_mislabeled_negative_proof_is_not_a_deadline_failure() {
    let mut registry = Registry::default();
    registry.accept(declaration("roll", &["host-a"]), START).unwrap();
    for variant in ["missing", "empty-path", "empty-revision", "exact-target"] {
        let mut receipt = observation("roll", "host-a", START + DAY + 1, Adoption::NotAdopted);
        if let Receipt::HostObserved { evidence, .. } = &mut receipt {
            match variant {
                "missing" => *evidence = None,
                "empty-path" => evidence.as_mut().unwrap().home_path.clear(),
                "empty-revision" => evidence.as_mut().unwrap().daemon_revision.clear(),
                "exact-target" => evidence.as_mut().unwrap().daemon_ready = true,
                _ => unreachable!(),
            }
        }
        assert!(registry.accept(receipt, START + 2 * DAY).is_err(), "{variant}");
        assert_eq!(sample(&registry, "roll", START + 2 * DAY), None);
    }
    registry.accept(observation("roll", "host-a", START + DAY - 1, Adoption::NotAdopted), START + 2 * DAY).unwrap();
    assert_eq!(sample(&registry, "roll", START + 2 * DAY), None, "stale predeadline proof is not a miss");
    registry.accept(observation("roll", "host-a", START + DAY + 1, Adoption::NotAdopted), START + 2 * DAY).unwrap();
    assert_eq!(sample(&registry, "roll", START + 2 * DAY), Some((START + DAY, true)));
}
