use super::*;

const START: u64 = 40 * DAY;
fn target() -> Identity {
    Identity { dotfiles_revision: "1111111111111111111111111111111111111111".into(),
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
            daemon_ready: adoption == Adoption::Verified, daemon_revision: target().daemon_revision })) }
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
    for field in ["daemon", "ready", "system", "home", "expected", "absent", "target"] {
        let mut receipt = observation("roll", "host-a", START + 1, Adoption::Verified);
        if let Receipt::HostObserved { evidence, target, .. } = &mut receipt {
            let proof = evidence.as_mut().unwrap();
            match field {
                "daemon" => proof.daemon_revision.truncate(7),
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
    complete_source_fixture(&mut registry, START + 101);
    let row = render(&registry, true, START + 101);
    assert_eq!(row["measurement"]["windows"][0]["eligible"], 1.0);
    assert_eq!(row["measurement"]["unknown_open_rolls"], 1);
    assert_eq!(row["remaining_percent"], 100.0);
    assert_eq!(row["measurement"]["external_population_coverage"], "complete-at-cut");
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
        assert_eq!(row(&store, now)["status"], "no-data");
    }
    let store = Store::open(&graph, "host-a").unwrap();
    setup_signer(&store);
    install(&store, directory.path()).unwrap();
    assert_eq!(ingest_fixture(&store, observation("roll", "host-a", now, Adoption::Verified), now + 1).unwrap()["changed"], false);
    assert_eq!(row(&store, now)["measurement"]["windows"][0]["bad"], 1.0);
    assert_eq!(row(&store, START + DAY)["measurement"]["windows"][0]["eligible"], 0.0);
    assert!(ingest_fixture(&store, declaration("roll", &["host-b"]), now + 1).is_err());
    store.read_snapshot(|_| { assert_eq!(row(&store, now)["status"], "no-data"); Ok(()) }).unwrap();
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
    complete_source_fixture(&mut registry, START + DAY + 60_000);
    assert_eq!(render(&registry, true, START + DAY + 60_000)["status"], "burning");
    // The 30m short window loses the bad event while the 6h long window retains it.
    complete_source_fixture(&mut registry, START + DAY + 1_800_001);
    assert_eq!(render(&registry, true, START + DAY + 1_800_001)["status"], "not-eligible");
}

#[tokio::test]
async fn api_ingestion_publishes_immediately_and_signals_native_readers() {
    use axum::{Json, extract::State};
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::tests_shared::state(root.path());
    install(&state.store, root.path()).unwrap();
    let mut changes = state.event_notify.subscribe();
    let id = publish_fixture(&state.store, declaration("roll", &["host-a"]), Some("agent/example/roll-authority"));
    let accepted = crate::api::operations::ingest_roll(
        State(state.clone()), Json(SignedReceipt { claim_id: id })).await.unwrap().0;
    assert_eq!(accepted["changed"], true);
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    let rows = crate::api::operations::current_slo_rows(&state.store, root.path()).unwrap();
    let row = rows.iter().find(|row| row["id"] == OBJECTIVE).unwrap();
    assert_eq!(row["measurement"]["accepted_declarations"], 1);
    assert_eq!(row["measurement"]["unknown_open_rolls"], 1);
    let id = publish_fixture(&state.store, declaration("roll", &["host-b"]), Some("agent/example/roll-authority"));
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
    store.admit_fleet_anchor("7c9e6679-7425-40de-944b-e07fc1f90ae7", KEY.public(), "listening").unwrap();
    // Slash-qualified identity is placement-independent. A simple name becomes
    // agent/{host}.roll-authority and cannot authorize the separately signed principal.
    let source = format!(r#"version 2
agent "example/roll-authority" {{
  command "true"
  host "{}"
  resource "roll-authority" subject="resource/fleet/roll-completion" reason="Declare rolls"
}}
"#, store.origin());
    if !store.declared_resource_referrers(RESOURCE).unwrap().iter().any(|binding|
        binding["owner"] == "agent/example/roll-authority" && binding["name"] == "roll-authority")
    {
        let intent = crate::graph::parse_intent(&source, store.origin()).unwrap();
        let preview = store.mission(&intent, crate::model::IntentInput { kdl: source, source_name: None }).unwrap();
        // Re-enabling a removed binding is a new publication, not a replay of
        // the first source-hash idempotency key.
        let key = format!("roll-authority:{}:{}", intent.source_hash, store.index().unwrap());
        store.apply(&intent, &preview.subject_tokens, &key).unwrap();
    }
    store.ensure_principal_key("agent/example/roll-authority").unwrap();
}

pub(crate) fn publish_fixture(store: &Store, receipt: Receipt, actor: Option<&str>) -> String {
    setup_signer(store);
    if let Some(actor) = actor { store.ensure_principal_key(actor).unwrap(); }
    let subject = match &receipt {
        Receipt::RollDeclared { roll_id, .. } => format!("{RESOURCE}/{roll_id}/declaration"),
        Receipt::HostTargetResolved { roll_id, host, .. } => format!("{RESOURCE}/{roll_id}/{host}/resolution"),
        Receipt::HostObserved { roll_id, host, observed_at_ms, .. } => format!("{RESOURCE}/{roll_id}/{host}/{observed_at_ms}"),
        Receipt::IntentObserved { roll_id, host, intent_claim_id, .. } => format!("{RESOURCE}/{roll_id}/intents/{host}/{intent_claim_id}"),
        Receipt::SourceEnabled { producer, .. } => format!("{RESOURCE}/sources/{producer}/enabled"),
        Receipt::SourceFrontier { producer, host, hosts, .. } => format!("{RESOURCE}/sources/{producer}/{host}/frontier/{}", hosts.get(host).unwrap()),
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
        let id = publish_fixture(store, resolution, Some("agent/example/roll-authority"));
        ingest(store, SignedReceipt { claim_id: id }, now)?;
    }
    let actor = if matches!(receipt, Receipt::HostObserved { .. } | Receipt::SourceFrontier { .. }) { None } else { Some("agent/example/roll-authority") };
    let id = publish_fixture(store, receipt, actor);
    ingest(store, SignedReceipt { claim_id: id }, now)
}

#[test]
fn declared_native_authority_binds_the_exact_canonical_signer() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    setup_signer(&store);
    assert_eq!(store.declared_resource_referrers(RESOURCE).unwrap(), vec![json!({
        "owner": "agent/example/roll-authority", "name": "roll-authority", "reason": "Declare rolls"
    })]);
    // A verified key with a similar but undeclared identity is not the bound role.
    let unbound = publish_fixture(&store, declaration("canonical", &["host-a"]), Some("agent/roll-authority"));
    assert_eq!(store.claim_authorization(&unbound).unwrap().verdict, smallclaims::principal::Verdict::Verified);
    assert!(ingest(&store, SignedReceipt { claim_id: unbound }, START).is_err());
    let declared = publish_fixture(&store, declaration("canonical", &["host-a"]), Some("agent/example/roll-authority"));
    assert!(!dependencies_pending(&store, &declared).unwrap());
    ingest(&store, SignedReceipt { claim_id: declared }, START).unwrap();
    assert_eq!(row(&store, START)["measurement"]["accepted_declarations"], 1);
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
    let forged = publish_fixture(&store, observation("roll", "host-a", START + 1, Adoption::Verified), Some("agent/example/roll-authority"));
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
    for actor in [None, Some("agent/example/roll-authority")] {
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
        "actor":"agent/example/roll-authority","fields":{"kind":"custom.fractal.fleet-roll-receipt","state":"observed",
        "facts":observation("poison", "node", START + 1, Adoption::Verified)}});
    let response = app.clone().oneshot(Request::builder().method("POST").uri("/v1/claims")
        .extension(crate::api::BoundAgent("agent/example/roll-authority".into()))
        .header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert!(!response.status().is_success());
    // The same bound producer may publish and admit a legitimate declaration after spoof attempts.
    let body = json!({"subject":format!("{RESOURCE}/poison/declaration"),"kind":"resource.observed",
        "actor":"agent/example/roll-authority","fields":{"kind":"custom.fractal.fleet-roll-receipt","state":"declared",
        "facts":declaration("poison", &["node"])}});
    let response = app.oneshot(Request::builder().method("POST").uri("/v1/claims")
        .extension(crate::api::BoundAgent("agent/example/roll-authority".into()))
        .header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert!(response.status().is_success());
    let claim: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    state.store.seal_local_batches().unwrap();
    state.store.judge_claims(true).unwrap();
    let signed = SignedReceipt { claim_id: claim["value"]["id"].as_str().unwrap().into() };
    assert_eq!(ingest(&state.store, signed, START + 1).unwrap()["changed"], true);
}

#[test]
fn removed_factory_revision_is_rejected_in_every_nested_identity_and_proof_even_null() {
    for removed in [Value::Null, json!("obsolete-factory-revision")] {
        let mut target = serde_json::to_value(target()).unwrap();
        target["factory_revision"] = removed.clone();
        assert!(serde_json::from_value::<Identity>(target.clone()).is_err());
        let mut declared = serde_json::to_value(declaration("roll", &["host-a"])).unwrap();
        declared["targets"]["host-a"] = target.clone();
        assert!(serde_json::from_value::<Receipt>(declared).is_err());
        let mut observed = serde_json::to_value(observation("roll", "host-a", START + 1, Adoption::Verified)).unwrap();
        observed["target"] = target;
        assert!(serde_json::from_value::<Receipt>(observed).is_err());
        let mut observed = serde_json::to_value(observation("roll", "host-a", START + 1, Adoption::Verified)).unwrap();
        observed["evidence"]["factory_revision"] = removed;
        assert!(serde_json::from_value::<Verification>(observed["evidence"].clone()).is_err());
        assert!(serde_json::from_value::<Receipt>(observed).is_err());
    }
}

#[test]
fn resolution_requires_the_original_authority_and_immutable_admitted_declaration() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let declared = publish_fixture(&store, declaration("roll", &["host-a"]), Some("agent/example/roll-authority"));
    ingest(&store, SignedReceipt { claim_id: declared.clone() }, START).unwrap();
    let resolution = Receipt::HostTargetResolved { version: 1, roll_id: "roll".into(), declared_at_ms: START,
        host: "host-a".into(), target: target(), expected_system_path: "/nix/store/example-system".into(),
        expected_home_path: "/nix/store/example-home".into(), declaration_claim_id: declared };
    let forged = publish_fixture(&store, resolution.clone(), Some("agent/other"));
    assert!(ingest(&store, SignedReceipt { claim_id: forged }, START).is_err());
    let legitimate = publish_fixture(&store, resolution.clone(), Some("agent/example/roll-authority"));
    ingest(&store, SignedReceipt { claim_id: legitimate.clone() }, START).unwrap();
    assert_eq!(resolved_target(&store, &legitimate).unwrap().host, "host-a");
    assert_eq!(ingest(&store, SignedReceipt { claim_id: legitimate }, START + 1).unwrap()["changed"], false);
    let mut conflict = resolution;
    if let Receipt::HostTargetResolved { expected_home_path, .. } = &mut conflict {
        *expected_home_path = "/nix/store/different-home".into();
    }
    let conflict = publish_fixture(&store, conflict, Some("agent/example/roll-authority"));
    assert!(ingest(&store, SignedReceipt { claim_id: conflict }, START + 1).is_err());
}

#[test]
fn signed_host_proof_survives_canonical_graph_replication_without_trusting_origin_text() {
    const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    let root = tempfile::tempdir().unwrap();
    let source = Store::open_memory("host-a").unwrap();
    install(&source, root.path()).unwrap();
    let declared = publish_fixture(&source, declaration("replicated", &["host-a"]), Some("agent/example/roll-authority"));
    ingest(&source, SignedReceipt { claim_id: declared.clone() }, START).unwrap();
    let resolved = publish_fixture(&source, Receipt::HostTargetResolved {
        version: 1, roll_id: "replicated".into(), declared_at_ms: START, host: "host-a".into(), target: target(),
        expected_system_path: "/nix/store/example-system".into(), expected_home_path: "/nix/store/example-home".into(),
        declaration_claim_id: declared.clone(),
    }, Some("agent/example/roll-authority"));
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
    assert!(replica.claim_has_authenticated_writer(&observed, "host-a").unwrap());
    assert!(!replica.claim_has_authenticated_writer(&observed, "host-b").unwrap());
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
    store.revoke_device_key("agent/example/roll-authority", &signature.key, "example revocation").unwrap();
    store.seal_local_batches().unwrap();
    store.judge_claims(true).unwrap();
    assert_eq!(store.claim_authorization(declared["claim_id"].as_str().unwrap()).unwrap().verdict,
        smallclaims::principal::Verdict::Verified, "historical authenticity is not current roll authority");
    rebuild(&store).unwrap();
    assert_eq!(row(&store, START + 1)["measurement"]["windows"][0]["eligible"], 0.0);
    assert_eq!(row(&store, START + 1)["status"], "no-data");
}

// Reducer fixtures represent already admitted native proofs. API/signature tests
// below exercise the real publication boundary; no production eligibility bypass.
fn complete_source_fixture(registry: &mut Registry, through: u64) {
    let producer = "agent/example/roll-authority".to_owned();
    let host = "host-a".to_owned();
    registry.membership = Membership { producers: BTreeSet::from([producer.clone()]),
        hosts: BTreeMap::from([(host.clone(), "incarnation-a".into())]), since_ms: 0 };
    record_source(registry, Receipt::SourceEnabled { version: 1, producer: producer.clone(),
        enabled_at_ms: 0, mission_prefix: "demo/roll/".into(), resource_input: "main".into(),
        revision_fact: "head".into(), roll_id_prefix: "demo/".into() }, "enable", &producer, 0);
    let hosts = registry.membership.hosts.clone();
    record_source(registry, Receipt::SourceFrontier { version: 1, producer, host,
        members: vec!["agent/example/roll-authority".into()], hosts,
        source_claim_id: "enable".into(), covered_from_ms: 0, through_ms: through,
        previous_frontier_claim_id: None, source_index: 1, source_lifetime: "database:1".into(), receipt_claim_ids: vec![] },
        &format!("frontier-{through}"), "host/host-a", through);
}

#[test]
fn missing_population_proof_never_labels_known_good_or_bad_adoption_as_fleet_health() {
    for adoption in [Adoption::Verified, Adoption::NotAdopted] {
        let mut registry = Registry::default();
        registry.accept(declaration("roll", &["host-a"]), START).unwrap();
        registry.accept(observation("roll", "host-a", START + DAY + 1, adoption), START + DAY + 1).unwrap();
        let row = render(&registry, true, START + DAY + 2);
        assert_eq!(row["status"], "no-data");
        assert!(row["remaining_percent"].is_null());
        assert!(row["measurement"]["population_frontier_ms"].is_null());
        assert_eq!(row["measurement"]["windows"][0]["bad"], 1.0);
        assert_eq!(row["measurement"]["external_population_coverage"], "unknown");
    }
}

#[test]
fn common_cut_requires_every_producer_every_host_and_current_incarnation() {
    let now = START + DAY + 10;
    let mut registry = Registry::default();
    complete_source_fixture(&mut registry, now - 1);
    assert_eq!(source_cut(&registry, now).unwrap().through_ms, now - 1);
    assert_eq!(render(&registry, true, now)["measurement"]["uncovered_tail_ms"], 1);
    registry.membership.hosts.insert("host-b".into(), "incarnation-b".into());
    assert!(source_cut(&registry, now).is_none());
    registry.membership.hosts.remove("host-b");
    registry.membership.hosts.insert("host-a".into(), "rejoined-incarnation".into());
    assert!(source_cut(&registry, now).is_none());
    complete_source_fixture(&mut registry, now);
    registry.membership.producers.insert("agent/other-release-driver".into());
    assert!(source_cut(&registry, now).is_none());
}

#[test]
fn stale_or_short_history_retains_diagnostics_without_budget_eligibility() {
    let now = START + DAY + 10;
    let mut registry = Registry::default();
    registry.accept(declaration("roll", &["host-a"]), START).unwrap();
    registry.accept(observation("roll", "host-a", now - 1, Adoption::Verified), now - 1).unwrap();
    complete_source_fixture(&mut registry, now);
    assert_eq!(render(&registry, true, now)["status"], "exhausted");
    let stale = render(&registry, true, now + FRONTIER_FRESHNESS_MS + 1);
    assert_eq!(stale["status"], "no-data");
    assert_eq!(stale["measurement"]["windows"][0]["bad"], 1.0);
    assert_eq!(stale["measurement"]["windows"][0]["budget_eligible"], false);
    registry.source_accepted_at.insert("agent/example/roll-authority".into(), now - DAY);
    assert_eq!(render(&registry, true, now)["status"], "no-data");
    assert_eq!(render(&registry, true, now)["measurement"]["windows"][0]["budget_eligible"], false);
}

#[test]
fn frontier_waits_for_positive_receipt_and_its_predecessor_before_source_admission() {
    let now = START + DAY + 10;
    let mut registry = Registry::default();
    complete_source_fixture(&mut registry, now - 2);
    let receipt = Receipt::SourceFrontier { version: 1, producer: "agent/example/roll-authority".into(),
        host: "host-a".into(), members: vec!["agent/example/roll-authority".into()],
        hosts: registry.membership.hosts.clone(), source_claim_id: "enable".into(),
        covered_from_ms: 0, through_ms: now, previous_frontier_claim_id: Some(format!("frontier-{}", now - 2)),
        source_index: 2, source_lifetime: "database:1".into(), receipt_claim_ids: vec!["positive-host-observation".into()] };
    assert!(validate_source(&registry, &receipt, now).is_err());
    registry.admitted_ids.insert("positive-host-observation".into());
    assert!(validate_source(&registry, &receipt, now).unwrap());
    registry.frontier_ids.clear();
    assert!(validate_source(&registry, &receipt, now).is_err());
}

#[tokio::test]
async fn generic_claim_api_never_signs_source_frontiers_even_for_bound_authority() {
    use axum::{body::{Body, to_bytes}, http::Request};
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::tests_shared::state(root.path());
    setup_signer(&state.store);
    install(&state.store, root.path()).unwrap();
    let app = crate::api::router(state.clone());
    for actor in [None, Some("agent/example/roll-authority")] {
        let body = json!({"subject":format!("{RESOURCE}/sources/agent/example/roll-authority/node/frontier/1"),
            "kind":"resource.observed","actor":actor,
            "fields":{"kind":"custom.fractal.fleet-roll-receipt","state":"source-closed",
                "facts":{"version":1,"kind":"source-frontier","producer":"agent/example/roll-authority",
                    "host":"node","hosts":{},"members":["agent/example/roll-authority"],
                    "source_claim_id":"invented","covered_from_ms":0,"through_ms":1,
                    "previous_frontier_claim_id":null,"source_index":1,"receipt_claim_ids":[]}}});
        let response = app.clone().oneshot(Request::builder().method("POST").uri("/v1/claims")
            .extension(crate::api::BoundAgent("agent/example/roll-authority".into()))
            .header("content-type","application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
        assert!(!response.status().is_success());
    }
    let enabled = Receipt::SourceEnabled { version:1,producer:"agent/example/roll-authority".into(),
        enabled_at_ms:0,mission_prefix:"demo/roll/".into(),resource_input:"main".into(),
        revision_fact:"head".into(),roll_id_prefix:"demo/".into() };
    let body = json!({"subject":format!("{RESOURCE}/sources/agent/example/roll-authority/enabled"),
        "kind":"resource.observed","actor":"agent/example/roll-authority",
        "fields":{"kind":"custom.fractal.fleet-roll-receipt","state":"enabled","facts":enabled}});
    let response = app.oneshot(Request::builder().method("POST").uri("/v1/claims")
        .extension(crate::api::BoundAgent("agent/example/roll-authority".into()))
        .header("content-type","application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert!(response.status().is_success());
    let claim: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    state.store.seal_local_batches().unwrap();
    state.store.judge_claims(true).unwrap();
    let id = claim["value"]["id"].as_str().unwrap().to_owned();
    let accepted = state.store.claim_by_id(&id).unwrap().unwrap().accepted_at_unix_ms as u64;
    ingest(&state.store, SignedReceipt { claim_id:id }, START).unwrap();
    assert!(source_configs(&state.store).unwrap()[0].enabled_at_ms >= accepted,
        "a backdated caller journal cannot manufacture historical native coverage");
}

pub(crate) fn native_source_fixture(store: &Store) -> SourceConfig {
    setup_signer(store);
    let claim = store.append_claim(&crate::model::ClaimInput {
        subject: "resource/demo/anchor".into(), kind: "resource.observed".into(), actor: None,
        fields: BTreeMap::from([("kind".into(), json!("custom.demo.anchor")), ("state".into(), json!("ready"))]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    store.seal_local_batches().unwrap();
    store.judge_claims(true).unwrap();
    let key = store.claim_authorization(&claim.id).unwrap().signature.unwrap().key;
    store.admit_fleet_anchor("7c9e6679-7425-40de-944b-e07fc1f90ae7", &key, "listening").unwrap();
    store.judge_claims(true).unwrap();
    let id = publish_fixture(store, Receipt::SourceEnabled { version: 1, producer: "agent/example/roll-authority".into(),
        enabled_at_ms: 0, mission_prefix: "demo/roll/".into(), resource_input: "main".into(),
        revision_fact: "head".into(), roll_id_prefix: "demo/".into() }, Some("agent/example/roll-authority"));
    let now = store.claim_by_id(&id).unwrap().unwrap().accepted_at_unix_ms as u64;
    ingest(store, SignedReceipt { claim_id: id }, now).unwrap();
    source_configs(store).unwrap().pop().unwrap()
}

#[test]
fn signed_frontier_cannot_outrun_positive_adoption_admission_and_survives_replay() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let config = native_source_fixture(&store);
    ingest_fixture(&store, declaration("receipt-race", &["host-a"]), START).unwrap();
    ingest_fixture(&store, observation("receipt-race", "host-a", START + DAY + 1, Adoption::NotAdopted),
        START + DAY + 1).unwrap();
    let positive = publish_fixture(&store,
        observation("receipt-race", "host-a", START + DAY - 1, Adoption::Verified), None);
    let canonical = membership(&store).unwrap();
    let through = config.enabled_at_ms + 10_000;
    store.set_write_clock_at(u128::from(through + 1)).unwrap();
    let frontier = publish_fixture(&store, Receipt::SourceFrontier {
        version: 1, producer: config.producer, host: "host-a".into(),
        hosts: canonical.hosts, members: canonical.producers.into_iter().collect(),
        source_claim_id: config.source_claim_id, covered_from_ms: config.enabled_at_ms,
        through_ms: through, previous_frontier_claim_id: None, source_index: store.index().unwrap(),
        source_lifetime: crate::roll_source::source_lifetime(&store).unwrap(),
        receipt_claim_ids: vec![positive.clone()],
    }, None);
    assert!(ingest(&store, SignedReceipt { claim_id: frontier.clone() }, through).is_err());
    assert!(dependencies_pending(&store, &frontier).unwrap());
    {
        let source = store.fractal.extensions.get::<Source>();
        let registry = source.registry.read().unwrap();
        assert!(source_cut(&registry, through).is_none());
        assert_eq!(sample(&registry, "receipt-race", START + DAY + 2), Some((START + DAY, true)));
    }
    ingest(&store, SignedReceipt { claim_id: positive }, through).unwrap();
    ingest(&store, SignedReceipt { claim_id: frontier.clone() }, through).unwrap();
    rebuild(&store).unwrap();
    let source = store.fractal.extensions.get::<Source>();
    let registry = source.registry.read().unwrap();
    assert!(registry.frontier_ids.contains_key(&frontier));
    assert_eq!(source_cut(&registry, through).unwrap().through_ms, through);
    assert_eq!(sample(&registry, "receipt-race", START + DAY + 2), Some((START + DAY - 1, false)));
}

#[test]
fn native_frontier_serializes_behind_snapshot_races_and_fences_cached_old_clock_writes() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let config = native_source_fixture(&store);
    let captured = store.index().unwrap();
    let through = config.enabled_at_ms + 10_000;
    // This accepted source event was absent from the snapshot but has an older
    // canonical acceptance clock. A frontier must not certify the captured cut.
    let raced = publish_fixture(&store, declaration("queued-old-clock", &["host-a"]), Some("agent/example/roll-authority"));
    assert!(publish_frontier(&store, &config, config.enabled_at_ms, through, captured, &crate::roll_source::source_lifetime(&store).unwrap(), through + 1).is_err());
    ingest(&store, SignedReceipt { claim_id: raced }, through).unwrap();
    let captured = store.index().unwrap();
    store.set_write_clock_at(u128::from(through + 1)).unwrap();
    let token = authority_cut(&store).unwrap();
    publish_frontier(&store, &config, config.enabled_at_ms, through, captured, &crate::roll_source::source_lifetime(&store).unwrap(), through + 1).unwrap();
    assert_eq!(authority_cut(&store).unwrap(), token, "verified quiet frontiers must not replay retained authority history");
    store.set_write_clock_at(1).unwrap();
    let later = publish_fixture(&store, declaration("after-fence", &["host-a"]), Some("agent/example/roll-authority"));
    assert!(store.claim_by_id(&later).unwrap().unwrap().accepted_at_unix_ms > u128::from(through),
        "a writer prepared with an old cached clock cannot backdate behind a sealed source cut");
}

#[test]
fn native_receipt_backlog_chunks_without_truncating_same_millisecond_coverage() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let config = native_source_fixture(&store);
    store.set_write_clock_at(u128::from(config.enabled_at_ms)).unwrap();
    let mut ids = Vec::new();
    for index in 0..513 {
        let roll = format!("chunk-{index}");
        ids.push(store.append_claim(&crate::model::ClaimInput {
            subject: format!("{RESOURCE}/{roll}/declaration"), kind: "resource.observed".into(),
            actor: Some("agent/example/roll-authority".into()),
            fields: BTreeMap::from([("kind".into(), json!("custom.fractal.fleet-roll-receipt")),
                ("state".into(), json!("declared")), ("facts".into(), serde_json::to_value(declaration(&roll, &["host-a"])).unwrap())]),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap().id);
    }
    store.seal_local_batches().unwrap();
    store.judge_claims(true).unwrap();
    let now = config.enabled_at_ms + 2_000;
    for id in ids { ingest(&store, SignedReceipt { claim_id: id }, now).unwrap(); }
    store.set_write_clock_at(u128::from(now)).unwrap();
    let first = publish_frontier(&store, &config, config.enabled_at_ms, now - 1_000, store.index().unwrap(), &crate::roll_source::source_lifetime(&store).unwrap(), now).unwrap();
    assert_eq!(first["more_receipts"], true);
    let first_claim = store.claim_by_id(first["claim_id"].as_str().unwrap()).unwrap().unwrap();
    assert_eq!(first_claim.body["fields"]["facts"]["receipt_claim_ids"].as_array().unwrap().len(), 512);
    {
        let source = store.fractal.extensions.get::<Source>();
        assert!(source_cut(&source.registry.read().unwrap(), now).is_none(),
            "an excluded receipt in the same onset millisecond leaves an empty interval, not false completeness");
    }
    let second = publish_frontier(&store, &config, config.enabled_at_ms, now - 1_000, store.index().unwrap(), &crate::roll_source::source_lifetime(&store).unwrap(), now).unwrap();
    assert_eq!(second["more_receipts"], false);
    let second_claim = store.claim_by_id(second["claim_id"].as_str().unwrap()).unwrap().unwrap();
    assert_eq!(second_claim.body["fields"]["facts"]["receipt_claim_ids"].as_array().unwrap().len(), 1);
    assert_eq!(first_claim.subject, second_claim.subject, "heartbeats version one stable incarnation resource");
    assert_eq!(second_claim.body["fields"]["facts"]["previous_frontier_claim_id"], first["claim_id"]);
    let source = store.fractal.extensions.get::<Source>();
    assert_eq!(source_cut(&source.registry.read().unwrap(), now).unwrap().through_ms, now - 1_000);
}

#[test]
fn equivalent_signed_enrollment_preserves_epoch_and_recovers_after_key_rotation_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("graph.sqlite3");
    let enrollment;
    let original;
    {
        let store = Store::open(&path, "host-a").unwrap();
        install(&store, root.path()).unwrap();
        original = native_source_fixture(&store);
        enrollment = serde_json::from_value::<Receipt>(store.claim_by_id(&original.source_claim_id)
            .unwrap().unwrap().body["fields"]["facts"].clone()).unwrap();
        let duplicate = publish_fixture(&store, enrollment.clone(), Some("agent/example/roll-authority"));
        assert_ne!(duplicate, original.source_claim_id);
        let now = store.claim_by_id(&duplicate).unwrap().unwrap().accepted_at_unix_ms as u64;
        assert_eq!(ingest(&store, SignedReceipt { claim_id: duplicate }, now).unwrap()["changed"], false);
        assert_eq!(source_configs(&store).unwrap()[0].source_claim_id, original.source_claim_id);
        assert_eq!(source_configs(&store).unwrap()[0].enabled_at_ms, original.enabled_at_ms);
        let key = store.claim_authorization(&original.source_claim_id).unwrap().signature.unwrap().key;
        store.revoke_device_key("agent/example/roll-authority", &key, "demo key rotation").unwrap();
        store.seal_local_batches().unwrap();
        store.judge_claims(true).unwrap();
        rebuild(&store).unwrap();
        assert!(source_configs(&store).unwrap().is_empty());
    }
    // A new bound producer key is minted by the existing native principal path,
    // not an actor override or private-key read. Original stable facts survive.
    let store = Store::open(&path, "host-a").unwrap();
    setup_signer(&store);
    install(&store, root.path()).unwrap();
    let renewed = publish_fixture(&store, enrollment, Some("agent/example/roll-authority"));
    let accepted = store.claim_by_id(&renewed).unwrap().unwrap().accepted_at_unix_ms as u64;
    ingest(&store, SignedReceipt { claim_id: renewed.clone() }, accepted).unwrap();
    rebuild(&store).unwrap();
    let current = source_configs(&store).unwrap().pop().unwrap();
    assert_eq!(current.source_claim_id, renewed);
    assert_ne!(current.source_claim_id, original.source_claim_id);
    assert!(current.enabled_at_ms >= accepted);
}

#[test]
fn three_signed_frontier_versions_wait_for_reversed_predecessor_admission() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let config = native_source_fixture(&store);
    let through = config.enabled_at_ms + 1_000;
    let now = through + 10_000;
    store.set_write_clock_at(u128::from(now)).unwrap();
    let mut ids = Vec::new();
    for offset in 0..3 {
        ids.push(publish_frontier(&store, &config, config.enabled_at_ms, through + offset,
            store.index().unwrap(), &crate::roll_source::source_lifetime(&store).unwrap(), now).unwrap()["claim_id"].as_str().unwrap().to_owned());
    }
    {
        // Model a mirror that has the signed graph versions but has not admitted
        // their source chain yet. Arrival order must never grant a frontier early.
        let source = store.fractal.extensions.get::<Source>();
        let mut registry = source.registry.write().unwrap();
        registry.frontiers.clear();
        registry.frontier_ids.clear();
        for id in &ids { registry.admitted_ids.remove(id); }
    }
    for id in ids.iter().rev().take(2) {
        assert!(ingest(&store, SignedReceipt { claim_id: id.clone() }, now).is_err());
        assert!(dependencies_pending(&store, id).unwrap());
    }
    for id in &ids { ingest(&store, SignedReceipt { claim_id: id.clone() }, now).unwrap(); }
    let source = store.fractal.extensions.get::<Source>();
    assert_eq!(source_cut(&source.registry.read().unwrap(), now).unwrap().through_ms, through + 2);
}

#[test]
fn current_frontier_retry_preserves_cut_and_superseded_predecessor_is_terminal() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let config = native_source_fixture(&store);
    let now = config.enabled_at_ms + 10_000;
    store.set_write_clock_at(u128::from(now)).unwrap();
    let lifetime = crate::roll_source::source_lifetime(&store).unwrap();
    let first = publish_frontier(&store, &config, config.enabled_at_ms, now - 2,
        store.index().unwrap(), &lifetime, now).unwrap()["claim_id"].as_str().unwrap().to_owned();
    let second = publish_frontier(&store, &config, config.enabled_at_ms, now - 1,
        store.index().unwrap(), &lifetime, now).unwrap()["claim_id"].as_str().unwrap().to_owned();
    let source = store.fractal.extensions.get::<Source>();
    let before = {
        let registry = source.registry.read().unwrap();
        assert!(!registry.frontier_ids.contains_key(&first));
        let proof = registry.frontiers.values().flat_map(BTreeMap::values).next().unwrap();
        let cut = source_cut(&registry, now).unwrap();
        (cut.covered_from_ms, cut.through_ms, proof.source_index)
    };
    assert_eq!(ingest(&store, SignedReceipt { claim_id: second.clone() }, now).unwrap()["changed"], false);
    assert!(ingest(&store, SignedReceipt { claim_id: first.clone() }, now).is_err());
    assert!(!dependencies_pending(&store, &first).unwrap());
    let registry = source.registry.read().unwrap();
    let proof = registry.frontiers.values().flat_map(BTreeMap::values).next().unwrap();
    let cut = source_cut(&registry, now).unwrap();
    assert_eq!((cut.covered_from_ms, cut.through_ms, proof.source_index), before);
    assert_eq!(proof.claim_id, second);
}

#[test]
fn earlier_timely_positive_after_late_positive_recomputes_whole_cohort_completion() {
    let mut registry = Registry::default();
    registry.accept(declaration("reordered-positive", &["host-a","host-b"]), START).unwrap();
    let late = START + 40 * 3_600_000;
    registry.accept(observation("reordered-positive", "host-a", late, Adoption::Verified), late).unwrap();
    registry.accept(observation("reordered-positive", "host-b", START + 22 * 3_600_000, Adoption::Verified), late).unwrap();
    assert_eq!(sample(&registry, "reordered-positive", late), Some((START + DAY, true)));
    let timely = START + 23 * 3_600_000;
    registry.accept(observation("reordered-positive", "host-a", timely, Adoption::Verified), late).unwrap();
    assert_eq!(sample(&registry, "reordered-positive", late), Some((timely, false)));
}

#[test]
fn source_database_epoch_change_does_not_reuse_old_arrival_cursor_or_coverage() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let config = native_source_fixture(&store);
    let now = config.enabled_at_ms + 10_000;
    store.set_write_clock_at(u128::from(now)).unwrap();
    let old = crate::roll_source::source_lifetime(&store).unwrap();
    publish_frontier(&store, &config, config.enabled_at_ms, now - 1, store.index().unwrap(), &old, now).unwrap();
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        smallclaims::ivm::events::rotate_identity(&transaction).unwrap();
        transaction.commit().unwrap();
    }
    let new = crate::roll_source::source_lifetime(&store).unwrap();
    assert_ne!(old,new);
    let reset = publish_frontier(&store, &config, config.enabled_at_ms, now - 1,
        store.index().unwrap(), &new, now).unwrap();
    let claim = store.claim_by_id(reset["claim_id"].as_str().unwrap()).unwrap().unwrap();
    assert!(claim.body["fields"]["facts"]["previous_frontier_claim_id"].is_null());
    assert_eq!(claim.body["fields"]["facts"]["covered_from_ms"], now);
    let source = store.fractal.extensions.get::<Source>();
    let registry = source.registry.read().unwrap();
    assert!(source_cut(&registry, now).is_none());
    assert_eq!(registry.frontier_ids.len(), 1);
    assert_eq!(registry.frontiers.values().map(BTreeMap::len).sum::<usize>(), 1,
        "old heartbeat maps stay in signed graph history, not the hot reducer");
}

#[test]
fn late_old_database_frontier_cannot_replace_newer_authenticated_native_writer_position() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open_memory("host-a").unwrap();
    install(&store, root.path()).unwrap();
    let config = native_source_fixture(&store);
    let now = config.enabled_at_ms + 10_000;
    store.set_write_clock_at(u128::from(now)).unwrap();
    let old_lifetime = crate::roll_source::source_lifetime(&store).unwrap();
    let old = publish_frontier(&store, &config, config.enabled_at_ms, now - 1, store.index().unwrap(),
        &old_lifetime, now).unwrap()["claim_id"].as_str().unwrap().to_owned();
    {
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        smallclaims::ivm::events::rotate_identity(&transaction).unwrap();
        transaction.commit().unwrap();
    }
    let new_lifetime = crate::roll_source::source_lifetime(&store).unwrap();
    let new = publish_frontier(&store, &config, config.enabled_at_ms, now - 1, store.index().unwrap(),
        &new_lifetime, now).unwrap()["claim_id"].as_str().unwrap().to_owned();
    assert!(ingest(&store, SignedReceipt { claim_id: old }, now).is_err());
    rebuild(&store).unwrap();
    let source = store.fractal.extensions.get::<Source>();
    let registry = source.registry.read().unwrap();
    assert!(registry.frontier_ids.contains_key(&new));
    assert_eq!(registry.frontiers.values().flat_map(BTreeMap::values).next().unwrap().source_lifetime, new_lifetime);
}

#[tokio::test]
async fn bound_public_declaration_api_rejects_removed_factory_field_including_null() {
    use axum::{body::Body,http::{Request,StatusCode}};
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::tests_shared::state(root.path());
    setup_signer(&state.store);
    let app = crate::api::router(state);
    for removed in [Value::Null,json!("obsolete-factory-revision")] {
        let mut facts = serde_json::to_value(declaration("removed-field", &["node"])).unwrap();
        facts["targets"]["node"]["factory_revision"] = removed;
        let body = json!({"subject":format!("{RESOURCE}/removed-field/declaration"),
            "kind":"resource.observed","actor":"agent/example/roll-authority",
            "fields":{"kind":"custom.fractal.fleet-roll-receipt","state":"declared","facts":facts}});
        let response = app.clone().oneshot(Request::builder().method("POST").uri("/v1/claims")
            .extension(crate::api::BoundAgent("agent/example/roll-authority".into()))
            .header("content-type","application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[test]
fn earlier_native_intent_refines_effective_onset_without_rewriting_immutable_declaration() {
    let mut registry = Registry::default();
    let declared = declaration("same-target", &["host-a"]);
    record_source(&mut registry, declared, "immutable-declaration", "agent/example/roll-authority", START);
    let verified = START + DAY - 3_600_000;
    registry.accept(observation("same-target", "host-a", verified, Adoption::Verified), verified).unwrap();
    assert_eq!(sample(&registry,"same-target",verified),Some((verified,false)));
    let earlier = START - 2 * 3_600_000;
    let onset = Receipt::IntentObserved { version:1,producer:"agent/example/roll-authority".into(),
        host:"host-b".into(),roll_id:"same-target".into(),source_claim_id:"source".into(),
        declaration_claim_id:"immutable-declaration".into(),intent_claim_id:"original-earlier-run".into(),intent_at_ms:earlier };
    registry.accept(onset.clone(),verified).unwrap();
    assert_eq!(registry.rolls["same-target"].declared_at_ms, START);
    assert_eq!(registry.rolls["same-target"].intent_onset_ms,earlier);
    assert_eq!(registry.declaration_ids["same-target"],"immutable-declaration");
    assert_eq!(sample(&registry,"same-target",verified),Some((earlier+DAY,true)));
    assert!(!registry.accept(onset,verified).unwrap());
    let later = Receipt::IntentObserved { version:1,producer:"agent/example/roll-authority".into(),
        host:"host-a".into(),roll_id:"same-target".into(),source_claim_id:"source".into(),
        declaration_claim_id:"immutable-declaration".into(),intent_claim_id:"original-later-run".into(),intent_at_ms:START+1 };
    registry.accept(later,verified).unwrap();
    assert_eq!(registry.rolls["same-target"].intent_onset_ms,earlier,"no later retry restarts the original native clock");
}

#[tokio::test]
async fn bound_public_claim_api_cannot_sign_native_intent_onset_even_with_real_authority() {
    use axum::{body::Body,http::Request};
    use tower::ServiceExt as _;
    let root = tempfile::tempdir().unwrap();
    let state = crate::api::tests_shared::state(root.path());
    setup_signer(&state.store);
    let app = crate::api::router(state);
    for actor in [None,Some("agent/example/roll-authority")] {
        let facts = Receipt::IntentObserved { version:1,producer:"agent/example/roll-authority".into(),
            host:"node".into(),roll_id:"forged-onset".into(),source_claim_id:"source".into(),
            declaration_claim_id:"declaration".into(),intent_claim_id:"invented-run".into(),intent_at_ms:0 };
        let body = json!({"subject":format!("{RESOURCE}/forged-onset/intents/node/invented-run"),
            "kind":"resource.observed","actor":actor,"fields":{"kind":"custom.fractal.fleet-roll-receipt",
                "state":"intent-observed","facts":facts}});
        let response = app.clone().oneshot(Request::builder().method("POST").uri("/v1/claims")
            .extension(crate::api::BoundAgent("agent/example/roll-authority".into()))
            .header("content-type","application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
        assert!(!response.status().is_success());
    }
}

#[test]
fn version_one_receipt_corpus_round_trips_every_existing_resource_fact_variant() {
    let receipts = [
        declaration("corpus", &["host-a"]),
        Receipt::HostTargetResolved { version:1,roll_id:"corpus".into(),declared_at_ms:START,
            host:"host-a".into(),target:target(),expected_system_path:"/nix/store/system".into(),
            expected_home_path:"/nix/store/home".into(),declaration_claim_id:"declaration".into() },
        observation("corpus","host-a",START+1,Adoption::Verified),
        Receipt::IntentObserved { version:1,producer:"agent/example/roll-authority".into(),host:"host-a".into(),
            roll_id:"corpus".into(),source_claim_id:"source".into(),declaration_claim_id:"declaration".into(),
            intent_claim_id:"native-creation".into(),intent_at_ms:START-1 },
        Receipt::SourceEnabled { version:1,producer:"agent/example/roll-authority".into(),enabled_at_ms:0,
            mission_prefix:"demo/roll/".into(),resource_input:"main".into(),revision_fact:"head".into(),roll_id_prefix:"demo/".into() },
        Receipt::SourceFrontier { version:1,producer:"agent/example/roll-authority".into(),host:"host-a".into(),
            hosts:BTreeMap::from([("host-a".into(),"public-key".into())]),members:vec!["agent/example/roll-authority".into()],
            source_claim_id:"source".into(),covered_from_ms:0,through_ms:START,previous_frontier_claim_id:None,
            source_index:1,source_lifetime:"database:1".into(),receipt_claim_ids:vec!["native-onset-proof".into()] },
    ];
    let mut kinds = BTreeSet::new();
    for receipt in receipts {
        let value = serde_json::to_value(&receipt).unwrap();
        kinds.insert(value["kind"].as_str().unwrap().to_owned());
        assert_eq!(serde_json::from_value::<Receipt>(value).unwrap(),receipt);
    }
    assert_eq!(kinds,BTreeSet::from(["roll-declared","host-target-resolved","host-observed",
        "intent-observed","source-enabled","source-frontier"].map(str::to_owned)));
}
