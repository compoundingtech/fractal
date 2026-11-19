use super::*;

const START: u64 = 10 * DAY;
fn target() -> Identity {
    Identity { dotfiles_revision: "1111111111111111111111111111111111111111".into(),
        factory_revision: Some("2222222222222222222222222222222222222222".into()),
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
        evidence: (adoption != Adoption::Unknown).then(|| Box::new(Verification {
            system_path: "/nix/store/example-system".into(), home_path: "/nix/store/example-home".into(),
            expected_system_path: "/nix/store/example-system".into(), expected_home_path: "/nix/store/example-home".into(),
            daemon_ready: adoption == Adoption::Verified, daemon_revision: target().daemon_revision, factory_revision: target().factory_revision })) }
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
                "factory" => proof.factory_revision = Some("old-factory".into()),
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
fn durable_restart_replays_signed_claim_references_and_preserves_accounting() {
    let directory = tempfile::tempdir().unwrap();
    let graph = directory.path().join("graph.sqlite3");
    let now = START + 2 * DAY;
    {
        let store = Store::open(&graph, "host-a").unwrap();
        install(&store, directory.path()).unwrap();
        ingest_fixture(&store, declaration("roll", &["host-a"]), START).unwrap();
        ingest_fixture(&store, observation("roll", "host-a", now, Adoption::Verified), now).unwrap();
        assert_eq!(row(&store, now)["status"], "exhausted");
    }
    let store = Store::open(&graph, "host-a").unwrap();
    setup_signer(&store);
    install(&store, directory.path()).unwrap();
    assert_eq!(ingest_fixture(&store, observation("roll", "host-a", now, Adoption::Verified), now + 1).unwrap()["changed"], false);
    assert_eq!(row(&store, now)["measurement"]["windows"][0]["bad"], 1.0);
    assert_eq!(row(&store, START + DAY)["measurement"]["windows"][0]["eligible"], 0.0);
    assert!(ingest_fixture(&store, declaration("roll", &["host-b"]), now + 1).is_err());
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
    let id = publish_fixture(&state.store, declaration("roll", &["host-a"]), Some("agent/roll-authority"));
    let accepted = crate::api::operations::ingest_roll(
        State(state.clone()), Json(SignedReceipt { claim_id: id })).await.unwrap().0;
    assert_eq!(accepted["changed"], true);
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    let rows = crate::api::operations::current_slo_rows(&state.store, root.path()).unwrap();
    let row = rows.iter().find(|row| row["id"] == OBJECTIVE).unwrap();
    assert_eq!(row["measurement"]["accepted_declarations"], 1);
    assert_eq!(row["measurement"]["unknown_open_rolls"], 1);
    let id = publish_fixture(&state.store, declaration("roll", &["host-b"]), Some("agent/roll-authority"));
    let error = crate::api::operations::ingest_roll(
        State(state), Json(SignedReceipt { claim_id: id })).await.unwrap_err();
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

pub(crate) fn setup_signer(store: &Store) {
    static KEY: std::sync::LazyLock<std::sync::Arc<smallclaims::fleet::MemberKey>> =
        std::sync::LazyLock::new(|| std::sync::Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0));
    store.bind_fleet("7c9e6679-7425-40de-944b-e07fc1f90ae7").unwrap();
    store.pin_fleet_anchor(KEY.public()).unwrap();
    store.set_member_key(Some(KEY.clone())).unwrap();
    let source = format!(r#"version 2
agent "roll-authority" {{
  command "true"
  host "{}"
  resource "roll-authority" subject="resource/fleet/roll-completion" reason="Declare rolls"
}}
"#, store.origin());
    let intent = crate::graph::parse_intent(&source, store.origin()).unwrap();
    let preview = store.mission(&intent, crate::model::IntentInput { kdl: source, source_name: None }).unwrap();
    store.apply(&intent, &preview.subject_tokens, &intent.source_hash).unwrap();
    store.ensure_principal_key("agent/roll-authority").unwrap();
}

pub(crate) fn publish_fixture(store: &Store, receipt: Receipt, actor: Option<&str>) -> String {
    setup_signer(store);
    if let Some(actor) = actor { store.ensure_principal_key(actor).unwrap(); }
    let subject = match &receipt {
        Receipt::RollDeclared { roll_id, .. } => format!("{RESOURCE}/{roll_id}/declaration"),
        Receipt::HostTargetResolved { roll_id, host, .. } => format!("{RESOURCE}/{roll_id}/{host}/resolution"),
        Receipt::HostObserved { roll_id, host, observed_at_ms, .. } => format!("{RESOURCE}/{roll_id}/{host}/{observed_at_ms}"),
    };
    let claim = store.append_claim(&crate::model::ClaimInput {
        subject, kind: "resource.observed".into(), actor: actor.map(str::to_owned),
        fields: BTreeMap::from([("kind".into(), json!("custom.fractal.fleet-roll-receipt")),
            ("state".into(), json!("observed")), ("facts".into(), serde_json::to_value(receipt).unwrap())]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    store.seal_local_batches().unwrap();
    store.judge_claims(true).unwrap();
    claim.id
}

pub(crate) fn ingest_fixture(store: &Store, receipt: Receipt, now: u64) -> Result<Value> {
    if let Receipt::HostObserved { roll_id, declared_at_ms, host, target, evidence: Some(proof), .. } = &receipt {
        let source = store.fractal.extensions.get::<Source>();
        let declaration_claim_id = source.registry.read().unwrap().declaration_ids.get(roll_id).unwrap().clone();
        let resolution = Receipt::HostTargetResolved { version: 1, roll_id: roll_id.clone(), declared_at_ms: *declared_at_ms,
            host: host.clone(), target: target.clone(), expected_system_path: proof.expected_system_path.clone(),
            expected_home_path: proof.expected_home_path.clone(), declaration_claim_id };
        let id = publish_fixture(store, resolution, Some("agent/roll-authority"));
        ingest(store, SignedReceipt { claim_id: id }, now)?;
    }
    let actor = if matches!(receipt, Receipt::HostObserved { .. }) { None } else { Some("agent/roll-authority") };
    let id = publish_fixture(store, receipt, actor);
    ingest(store, SignedReceipt { claim_id: id }, now)
}

#[test]
fn forged_declaration_cannot_predeclare_a_legitimate_roll_id() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let forged = publish_fixture(&store, declaration("roll", &["host-b"]), Some("agent/unbound"));
    assert!(ingest(&store, SignedReceipt { claim_id: forged }, START).is_err());
    ingest_fixture(&store, declaration("roll", &["host-a"]), START).unwrap();
    assert_eq!(row(&store, START)["measurement"]["accepted_declarations"], 1);
}

#[test]
fn signed_wrong_principal_and_cross_host_cannot_assert_adoption() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    ingest_fixture(&store, declaration("roll", &["host-a", "host-b"]), START).unwrap();
    let forged = publish_fixture(&store, observation("roll", "host-a", START + 1, Adoption::Verified), Some("agent/roll-authority"));
    assert!(ingest(&store, SignedReceipt { claim_id: forged }, START + 1).is_err());
    let foreign = publish_fixture(&store, observation("roll", "host-b", START + 1, Adoption::Verified), None);
    assert!(ingest(&store, SignedReceipt { claim_id: foreign }, START + 1).is_err());
    assert_eq!(row(&store, START + 1)["measurement"]["unknown_open_rolls"], 1);
}

#[tokio::test]
async fn public_claim_api_cannot_mint_host_evidence_or_unbound_declarations() {
    use axum::{body::{Body, to_bytes}, http::{Request, StatusCode}};
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::tests_shared::state(root.path());
    setup_signer(&state.store);
    install(&state.store, root.path()).unwrap();
    let app = crate::api::router(state.clone());
    let response = app.clone().oneshot(Request::builder().method("POST").uri("/v1/slo")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&declaration("raw-file", &["host-a"])).unwrap())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    for actor in [None, Some("agent/roll-authority")] {
        for receipt in [declaration("poison", &["host-a"]), observation("poison", "host-a", START + 1, Adoption::Verified)] {
            let body = json!({"subject":format!("{RESOURCE}/poison/host-a/1"), "kind":"resource.observed",
                "actor":actor, "fields":{"kind":"custom.fractal.fleet-roll-receipt","state":"observed","facts":receipt}});
            let response = app.clone().oneshot(Request::builder().method("POST").uri("/v1/claims")
                .header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN,
                "{}", String::from_utf8_lossy(&to_bytes(response.into_body(), usize::MAX).await.unwrap()));
        }
    }
    // Even the legitimate bound authority cannot ask the generic API to mint host proof.
    let body = json!({"subject":format!("{RESOURCE}/poison/node/1"),"kind":"resource.observed",
        "actor":"agent/roll-authority","fields":{"kind":"custom.fractal.fleet-roll-receipt","state":"observed",
        "facts":observation("poison", "node", START + 1, Adoption::Verified)}});
    let response = app.clone().oneshot(Request::builder().method("POST").uri("/v1/claims")
        .extension(crate::api::BoundAgent("agent/roll-authority".into()))
        .header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert!(!response.status().is_success());
    // The same bound producer may publish and admit a legitimate declaration after spoof attempts.
    let body = json!({"subject":format!("{RESOURCE}/poison/declaration"),"kind":"resource.observed",
        "actor":"agent/roll-authority","fields":{"kind":"custom.fractal.fleet-roll-receipt","state":"declared",
        "facts":declaration("poison", &["node"])}});
    let response = app.oneshot(Request::builder().method("POST").uri("/v1/claims")
        .extension(crate::api::BoundAgent("agent/roll-authority".into()))
        .header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert!(response.status().is_success());
    let claim: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    state.store.seal_local_batches().unwrap();
    state.store.judge_claims(true).unwrap();
    let signed = SignedReceipt { claim_id: claim["id"].as_str().unwrap().into() };
    assert_eq!(ingest(&state.store, signed, START + 1).unwrap()["changed"], true);
}

#[test]
fn omitted_factory_target_requires_no_factory_proof_but_explicit_target_does() {
    let mut registry = Registry::default();
    let mut declared = declaration("roll", &["host-a"]);
    if let Receipt::RollDeclared { targets, .. } = &mut declared { targets.get_mut("host-a").unwrap().factory_revision = None; }
    registry.accept(declared, START).unwrap();
    let mut observed = observation("roll", "host-a", START + 1, Adoption::Verified);
    if let Receipt::HostObserved { target, evidence, .. } = &mut observed {
        target.factory_revision = None;
        evidence.as_mut().unwrap().factory_revision = None;
    }
    assert!(registry.accept(observed, START + 1).is_ok());
    let mut explicit = Registry::default();
    explicit.accept(declaration("roll", &["host-a"]), START).unwrap();
    let mut missing = observation("roll", "host-a", START + 1, Adoption::Verified);
    if let Receipt::HostObserved { evidence, .. } = &mut missing { evidence.as_mut().unwrap().factory_revision = None; }
    assert!(explicit.accept(missing, START + 1).is_err());
}

#[test]
fn resolution_requires_the_original_authority_and_immutable_admitted_declaration() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let declared = publish_fixture(&store, declaration("roll", &["host-a"]), Some("agent/roll-authority"));
    ingest(&store, SignedReceipt { claim_id: declared.clone() }, START).unwrap();
    let resolution = Receipt::HostTargetResolved { version: 1, roll_id: "roll".into(), declared_at_ms: START,
        host: "host-a".into(), target: target(), expected_system_path: "/nix/store/example-system".into(),
        expected_home_path: "/nix/store/example-home".into(), declaration_claim_id: declared };
    let forged = publish_fixture(&store, resolution.clone(), Some("agent/other"));
    assert!(ingest(&store, SignedReceipt { claim_id: forged }, START).is_err());
    let legitimate = publish_fixture(&store, resolution.clone(), Some("agent/roll-authority"));
    ingest(&store, SignedReceipt { claim_id: legitimate.clone() }, START).unwrap();
    assert_eq!(resolved_target(&store, &legitimate).unwrap().host, "host-a");
    assert_eq!(ingest(&store, SignedReceipt { claim_id: legitimate }, START + 1).unwrap()["changed"], false);
    let mut conflict = resolution;
    if let Receipt::HostTargetResolved { expected_home_path, .. } = &mut conflict {
        *expected_home_path = "/nix/store/different-home".into();
    }
    let conflict = publish_fixture(&store, conflict, Some("agent/roll-authority"));
    assert!(ingest(&store, SignedReceipt { claim_id: conflict }, START + 1).is_err());
}

#[test]
fn signed_host_proof_survives_canonical_graph_replication_without_trusting_origin_text() {
    const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    let root = tempfile::tempdir().unwrap();
    let source = Store::open_memory("host-a").unwrap();
    install(&source, root.path()).unwrap();
    let declared = publish_fixture(&source, declaration("replicated", &["host-a"]), Some("agent/roll-authority"));
    ingest(&source, SignedReceipt { claim_id: declared.clone() }, START).unwrap();
    let resolved = publish_fixture(&source, Receipt::HostTargetResolved {
        version: 1, roll_id: "replicated".into(), declared_at_ms: START, host: "host-a".into(), target: target(),
        expected_system_path: "/nix/store/example-system".into(), expected_home_path: "/nix/store/example-home".into(),
        declaration_claim_id: declared.clone(),
    }, Some("agent/roll-authority"));
    ingest(&source, SignedReceipt { claim_id: resolved.clone() }, START).unwrap();
    let observed = publish_fixture(&source, observation("replicated", "host-a", START + 1, Adoption::Verified), None);
    let anchor = source.claim_authorization(&observed).unwrap().signature.unwrap().key;
    source.admit_fleet_anchor(FLEET, &anchor, "listening").unwrap();
    let member = std::sync::Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
    source.append_claim(&crate::model::ClaimInput {
        subject: "host/host-b".into(), kind: "fleet.member-admitted".into(), actor: None,
        fields: serde_json::from_value(json!({"fleet_id":FLEET,"member_key":member.public(),"via":"invite",
            "mode":"listening","sponsor":"host/host-a"})).unwrap(),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
    source.seal_local_batches().unwrap();
    let replica = Store::open_memory("host-b").unwrap();
    replica.bind_fleet(FLEET).unwrap();
    replica.pin_fleet_anchor(&anchor).unwrap();
    replica.set_member_key(Some(member)).unwrap();
    let exchange = source.export_replication_exchange_answering(FLEET,
        &replica.replication_inventory().unwrap(), &replica.replication_signature_requests().unwrap()).unwrap();
    replica.receive_replication_exchange("host-a", FLEET, &exchange).unwrap();
    replica.validate_replication_backlog().unwrap();
    replica.project_replication_backlog().unwrap();
    replica.judge_claims(true).unwrap();
    let local = tempfile::tempdir().unwrap();
    install(&replica, local.path()).unwrap();
    for id in [declared, resolved, observed] {
        ingest(&replica, SignedReceipt { claim_id: id }, START + 1).unwrap();
    }
    assert_eq!(row(&replica, START + 1)["measurement"]["windows"][0]["eligible"], 1.0);
    assert_eq!(row(&replica, START + 1)["measurement"]["windows"][0]["bad"], 0.0);
}

#[test]
fn revoked_declaration_proof_removes_cached_good_adoption_on_rebuild() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let declared = ingest_fixture(&store, declaration("roll", &["host-a"]), START).unwrap();
    ingest_fixture(&store, observation("roll", "host-a", START + 1, Adoption::Verified), START + 1).unwrap();
    assert_eq!(row(&store, START + 1)["measurement"]["windows"][0]["eligible"], 1.0);
    let signature = store.claim_authorization(declared["claim_id"].as_str().unwrap()).unwrap().signature.unwrap();
    store.revoke_device_key("agent/roll-authority", &signature.key, "example revocation").unwrap();
    store.seal_local_batches().unwrap();
    store.judge_claims(true).unwrap();
    rebuild(&store).unwrap();
    assert_eq!(row(&store, START + 1)["measurement"]["windows"][0]["eligible"], 0.0);
    assert_eq!(row(&store, START + 1)["status"], "no-data");
}
