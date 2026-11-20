use super::*;
use crate::fleet::MemberKey;
use crate::store::directive_notes::tests_shared_delivery::{FLEET, claim, enable, sync};

const ERROR: &str = "declared-admission-feature-barrier";
const PLAIN: &str = "version 2\nagent \"plain\" { workspace \"/tmp\"; command \"true\"; }\n";

fn publish(store: &Store, source: &str, key: &str) -> Result<ApplyResponse, FractalError> {
    let intent = crate::graph::parse_intent(source, &store.origin)?;
    let planned = store.mission(
        &intent,
        IntentInput {
            kdl: source.into(),
            source_name: None,
        },
    )?;
    store.apply_as(&intent, &planned.subject_tokens, key, Some("person/avery"))
}

fn started(store: &Store, feature: Value) {
    claim(
        store,
        &format!("daemon/{}", store.origin),
        "daemon.started",
        None,
        json!({"status":"running", "features":feature}),
    );
}

fn enabled() -> (Store, Arc<MemberKey>) {
    let store = Store::open_memory("alder").unwrap();
    let key = enable(&store);
    started(&store, json!({"declared_admission":1}));
    store.replication_snapshot().unwrap();
    (store, key)
}

fn peer(store: &Store, anchor: &MemberKey) -> (Store, Arc<MemberKey>) {
    let key = Arc::new(MemberKey::generate().unwrap().0);
    claim(
        store,
        "host/birch",
        "fleet.member-admitted",
        None,
        json!({"fleet_id":FLEET,"member_key":key.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}),
    );
    store.replication_snapshot().unwrap();
    let peer = Store::open_memory("birch").unwrap();
    peer.bind_fleet(FLEET).unwrap();
    peer.pin_fleet_anchor(anchor.public()).unwrap();
    peer.set_member_key(Some(key.clone())).unwrap();
    sync(store, &peer);
    (peer, key)
}

#[test]
fn recovery_observations_require_every_signed_fleet_reader() {
    use crate::store::resource_membership::{Admission, Coverage, DiskFit, Fit, LaneRequest, Owner};
    use fractal_runtime::{Capacity, RequestClass};
    let (store, anchor) = enabled();
    started(&store, json!({"resource_recovery_observations":1}));
    store.replication_snapshot().unwrap();
    let (peer, _) = peer(&store, &anchor);
    store.start_resource_authority("alder", "epoch").unwrap();
    let request = LaneRequest {
        id: "recovery".into(), name: "recovery".into(), host: "alder".into(),
        max_members: 1, capacity: Some(Capacity { cores: 1, memory: 1024 }),
        coverage: Coverage::Logical, admission_timeout_ms: None,
        class: RequestClass::Recovery, disk: None, io: None,
        profile: None, profile_estimate: None, disk_path: None,
        owner: Owner { run: "mission-run/recovery".into(), step: "step-run/recovery".into(),
            attempt: 1, invocation: None },
    };
    let fit = Fit { room: request.capacity, recovery_capable: true,
        disk: Some(DiskFit { filesystem: 1, available: 100, floor: 20 }), ..Fit::default() };
    for advertisement in [json!({}), json!({"resource_recovery_observations":0})] {
        started(&peer, advertisement);
        sync(&peer, &store);
        let before = store.index().unwrap();
        let error = store.request_resource_membership(&request, fit.clone(), 1).unwrap_err();
        assert_eq!(error.downcast_ref::<FractalError>().unwrap().code,
            "resource-recovery-feature-barrier");
        assert_eq!(store.index().unwrap(), before, "no observation or membership may be written");
    }
    started(&peer, json!({"resource_recovery_observations":1}));
    sync(&peer, &store);
    assert!(matches!(store.request_resource_membership(&request,
        Fit { held_bytes: 1024, ..fit.clone() }, 2).unwrap(), Admission::Queued { .. }));
    let mut queued = vec![request.clone()];
    for index in 0..crate::store::resource_admission_objectives::REPLAY_PAGE_SIZE {
        let mut extra = request.clone();
        extra.id = format!("recovery-{index:03}");
        extra.name = extra.id.clone();
        assert!(matches!(store.request_resource_membership(&extra,
            Fit { held_bytes: 1024, ..fit.clone() }, 2).unwrap(), Admission::Queued { .. }));
        queued.push(extra);
    }
    let mut refused = request.clone();
    refused.id = "refused-recovery".into();
    refused.name = refused.id.clone();
    assert!(matches!(store.request_resource_membership(&refused,
        Fit { recovery_capable: false, ..fit.clone() }, 3).unwrap(), Admission::Refused { .. }));
    started(&peer, json!({"resource_recovery_observations":0}));
    sync(&peer, &store);
    let coverage = || store.claims_by_kinds_after(
        &[crate::store::resource_admission_objectives::COVERAGE_KIND], None, 0, 1000)
        .unwrap().claims.last().unwrap().body["fields"]["complete"].clone();
    assert_eq!(coverage(), true);
    // No authority restart: withdrawal commits custody but withholds its terminal fact.
    for (index, member) in queued.iter().enumerate() {
        store.withdraw_resource_membership(&member.id, &member.owner, 4).unwrap();
        if index % 16 == 0 {
            // Continuous withheld traffic during repair must not reset or starve
            // previously pending IDs. They remain queued until readers recover.
            assert!(store.replay_resource_admission_observations().unwrap());
        }
    }
    assert_eq!(store.resource_membership_record(&request.id, &request.owner).unwrap().state, "retired");
    assert_eq!(coverage(), false);
    assert!(store.fractal.extensions.get::<crate::store::resource_admission_objectives::Coverage>().0
        .load(std::sync::atomic::Ordering::Acquire), "authority acquisition proof is still live");
    let observations = || store.claims_by_kinds_after(
        &[crate::store::resource_admission_objectives::KIND], None, 0, 1000).unwrap().claims;
    assert_eq!(observations().iter().rfind(|claim| claim.body["fields"]["id"] == request.id)
        .unwrap().body["fields"]["state"], "queued");
    assert!(store.replay_resource_admission_observations().unwrap());
    assert_eq!(coverage(), false, "a tick cannot certify while fleet support is absent");
    started(&peer, json!({"resource_recovery_observations":1}));
    sync(&peer, &store);
    assert_eq!(coverage(), false, "advertisement alone cannot repair stale evidence");
    // Idle authority maintenance, with no new membership request or restart:
    // one tick cannot replay more than the configured page.
    let before = observations().len();
    let validations = crate::store::resource_admission_objectives::reader_validation_count();
    assert!(store.replay_resource_admission_observations().unwrap());
    assert_eq!(crate::store::resource_admission_objectives::reader_validation_count() - validations, 1,
        "a full replay page must evaluate the fleet-reader barrier once, not once per ID");
    assert!(observations().len() - before <= crate::store::resource_admission_objectives::REPLAY_PAGE_SIZE);
    assert_eq!(coverage(), false);
    assert!(!store.replay_resource_admission_observations().unwrap());
    assert_eq!(crate::store::resource_admission_objectives::reader_validation_count() - validations, 2,
        "the final partial page validates its reader snapshot once");
    assert!(!store.replay_resource_admission_observations().unwrap());
    assert_eq!(crate::store::resource_admission_objectives::reader_validation_count() - validations, 2,
        "an idle complete queue must not fold fleet history");
    let events = observations();
    let retired = events.iter().rfind(|claim| claim.body["fields"]["id"] == request.id).unwrap();
    assert_eq!(retired.body["fields"]["state"], "retired");
    assert_eq!(coverage(), true);
    let cut = store.claims_by_kinds_after(
        &[crate::store::resource_admission_objectives::COVERAGE_KIND], None, 0, 1000).unwrap();
    assert!(retired.store_index < cut.claims.last().unwrap().store_index);
}

#[test]
fn absent_declarations_do_not_require_a_feature_barrier() {
    publish(&Store::open_memory("alder").unwrap(), PLAIN, "plain").unwrap();
}

#[test]
fn every_new_child_including_empty_budgets_requires_anchored_local_evidence() {
    for child in [
        "declared-priority \"normal\"",
        "declared-lane \"check\"",
        "declared-budgets {}",
    ] {
        for declaration in [
            format!(
                "version 2\nagent \"new\" {{ workspace \"/tmp\"; command \"true\"; {child}; }}\n"
            ),
            format!(
                "version 2\nmission \"new\" state=\"ready\" {{ goal \"Work.\"; {child}; step \"work\" {{ assigned-to \"agent/example/worker\"; goal \"Work.\"; }} }}\n"
            ),
        ] {
            let store = Store::open_memory("alder").unwrap();
            let before = store.index().unwrap();
            assert_eq!(
                publish(&store, &declaration, "unanchored")
                    .unwrap_err()
                    .code,
                ERROR
            );
            assert_eq!(store.index().unwrap(), before);
            enable(&store);
            store.replication_snapshot().unwrap();
            assert_eq!(
                publish(&store, &declaration, "missing").unwrap_err().code,
                ERROR
            );
            started(&store, json!({"declared_admission":0}));
            store.replication_snapshot().unwrap();
            assert_eq!(
                publish(&store, &declaration, "false").unwrap_err().code,
                ERROR
            );
            started(&store, json!({"declared_admission":1}));
            // A positive but unsealed advertisement is not anchored evidence.
            assert_eq!(
                publish(&store, &declaration, "unsealed").unwrap_err().code,
                ERROR
            );
            store.replication_snapshot().unwrap();
            publish(&store, &declaration, "supported").unwrap();
        }
    }
}

#[test]
fn old_member_never_receives_any_part_of_a_new_child_publication() {
    let (store, anchor) = enabled();
    let (peer, _) = peer(&store, &anchor);
    for advertisement in [json!({}), json!({"declared_admission":0})] {
        started(&peer, advertisement);
        sync(&peer, &store);
        let source = format!(
            "{PLAIN}mission \"new\" state=\"ready\" {{ goal \"Work.\"; declared-priority \"normal\"; step \"work\" {{ assigned-to \"agent/example/worker\"; goal \"Work.\"; }} }}\n"
        );
        let before = store.index().unwrap();
        assert_eq!(publish(&store, &source, "mixed").unwrap_err().code, ERROR);
        assert_eq!(
            store.index().unwrap(),
            before,
            "the whole publication must refuse before writes"
        );
        sync(&store, &peer);
        for target in [&store, &peer] {
            assert!(
                target
                    .selected_desired_token("agent/plain")
                    .unwrap()
                    .is_none()
            );
            assert!(target.mission_spec("new", None).unwrap().is_none());
        }
    }
    // A different origin cannot upgrade the old member on its behalf.
    claim(
        &store,
        "daemon/birch",
        "daemon.started",
        None,
        json!({"status":"running","features":{"declared_admission":1}}),
    );
    store.replication_snapshot().unwrap();
    let source = "version 2\nagent \"new\" { command \"true\"; declared-lane \"check\"; }\n";
    assert_eq!(publish(&store, source, "spoofed").unwrap_err().code, ERROR);
    started(&peer, json!({"declared_admission":1}));
    sync(&peer, &store);
    publish(&store, source, "upgraded").unwrap();
    // The latest false advertisement supersedes the positive one.
    started(&peer, json!({"declared_admission":0}));
    sync(&peer, &store);
    assert_eq!(
        publish(&store, source, "downgraded").unwrap_err().code,
        ERROR
    );
}

#[test]
fn direct_claim_publication_cannot_bypass_the_barrier() {
    let store = Store::open_memory("alder").unwrap();
    let source = "version 2\nagent \"example/new\" { command \"true\"; declared-lane \"check\"; }\nmission \"new\" state=\"ready\" { goal \"Work.\"; declared-priority \"normal\"; step \"work\" { assigned-to \"agent/example/worker\"; goal \"Work.\"; } }\n";

    let intent = crate::graph::parse_intent(source, "alder").unwrap();
    let desired = serde_json::to_value(&intent.subjects["agent/example/new"]).unwrap();
    let mission = serde_json::to_value(&intent.missions["new"]).unwrap();
    for (subject, kind, body) in [
        ("agent/example/new", "intent.desired", desired),
        ("mission/new", "mission.published", mission),
    ] {
        let input = ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: Some("person/avery".into()),
            fields: schema_fields_for_body(kind, &body).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        };
        let error = store.append_claim(&input).unwrap_err();
        assert_eq!(error.code, ERROR, "{kind}: {error}");
    }
}

#[test]
fn direct_declaration_claims_persist_metadata_with_complete_fleet_evidence() {
    let (store, anchor) = enabled();
    let (peer, _) = peer(&store, &anchor);
    started(&peer, json!({"declared_admission":1}));
    sync(&peer, &store);
    let intent = crate::graph::parse_intent(
        "version 2\nagent \"example/new\" { command \"true\"; declared-lane \"check\"; }\nmission \"new\" state=\"ready\" { goal \"Work.\"; declared-priority \"normal\"; step \"work\" { assigned-to \"agent/example/worker\"; goal \"Work.\"; } }\n",
        "alder",
    ).unwrap();
    for (subject, kind, body) in [
        ("agent/example/new", "intent.desired", serde_json::to_value(&intent.subjects["agent/example/new"]).unwrap()),
        ("mission/new", "mission.published", serde_json::to_value(&intent.missions["new"]).unwrap()),
    ] {
        let record = store.connection.batched(|transaction| {
            append_claim_tx(
                transaction, "alder", subject, kind, Some("person/avery"), &body, &[], None,
            )
        }).unwrap().unwrap();
        let persisted = claim_by_id_tx(&store.readers.get(), &record.id).unwrap().unwrap();
        assert_eq!(persisted.subject, subject);
        assert_eq!(persisted.kind, kind);
        assert_eq!(persisted.body, body);
        if kind == "intent.desired" {
            let desired: DesiredSubject = serde_json::from_value(persisted.body).unwrap();
            assert_eq!(desired.member.unwrap().declared_lane, Some(crate::model::DeclaredLane::Check));
        } else {
            let mission: MissionSpec = serde_json::from_value(persisted.body).unwrap();
            assert_eq!(mission.declared_priority, Some(crate::model::DeclaredPriority::Normal));
        }
    }
    // Raw append stores the declaration log. Replication selects domain projections.
    sync(&store, &peer);
    let (desired, _) = peer.desired_subject_with_writer("agent/example/new").unwrap().unwrap();
    assert_eq!(
        desired.member.unwrap().declared_lane,
        Some(crate::model::DeclaredLane::Check),
    );
    assert_eq!(
        peer.mission_spec("new", None).unwrap().unwrap().declared_priority,
        Some(crate::model::DeclaredPriority::Normal),
    );
}

#[test]
fn malformed_advertisement_signatures_refuse_like_unsigned_advertisements() {
    for malformed in [false, true] {
        let (store, anchor) = enabled();
        let (peer, _) = peer(&store, &anchor);
        started(&peer, json!({"declared_admission":1}));
        let mut exchange = peer.export_replication_exchange(
            FLEET, &store.replication_inventory().unwrap(),
        ).unwrap();
        assert!(!exchange.envelopes.is_empty());
        // Do not allow a detached valid signature to repair the damaged envelope.
        exchange.signatures.clear();
        for envelope in &mut exchange.envelopes {
            assert_eq!(envelope.writer, "birch");
            if malformed {
                envelope.signature.as_mut().unwrap().truncate(8);
            } else {
                envelope.signature = None;
            }
        }
        store.receive_replication_exchange("birch", FLEET, &exchange).unwrap();
        store.validate_replication_backlog().unwrap();
        store.apply_replication_repairs().unwrap();
        assert!(store.project_replication_backlog().unwrap());
        let source = "version 2\nagent \"new\" { command \"true\"; declared-lane \"check\"; }\n";
        let before = store.index().unwrap();
        assert_eq!(
            publish(&store, source, "invalid-advertisement").unwrap_err().code,
            ERROR,
            "malformed={malformed}",
        );
        assert_eq!(store.index().unwrap(), before);
        // The same advertisement works once the genuine incarnation signature arrives.
        sync(&peer, &store);
        publish(&store, source, "repaired-advertisement").unwrap();
    }
}

#[test]
fn pending_membership_and_unfenced_legacy_writers_fail_closed() {
    let (store, _) = enabled();
    let source = "version 2\nagent \"new\" { command \"true\"; declared-budgets {} }\n";
    let key = MemberKey::generate().unwrap().0;
    claim(
        &store,
        "host/birch",
        "fleet.member-admitted",
        None,
        json!({"fleet_id":FLEET,"member_key":key.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}),
    );
    assert_eq!(publish(&store, source, "pending").unwrap_err().code, ERROR);
    store.replication_snapshot().unwrap();
    assert_eq!(
        publish(&store, source, "missing-peer").unwrap_err().code,
        ERROR
    );

    let (store, _) = enabled();
    let legacy = Store::open_memory("legacy").unwrap();
    legacy.bind_fleet(FLEET).unwrap();
    started(&legacy, json!({"declared_admission":1}));
    sync(&legacy, &store);
    let error = publish(&store, source, "legacy").unwrap_err();
    assert_eq!(error.code, ERROR);
    assert!(error.message.contains("unfenced"));
}

#[test]
fn previous_incarnation_advertisements_do_not_enable_a_replacement() {
    let (store, anchor) = enabled();
    let (peer, key) = peer(&store, &anchor);
    started(&peer, json!({"declared_admission":1}));
    sync(&peer, &store);
    let high_water = writer_high_water(&peer.readers.get(), "birch")
        .unwrap()
        .unwrap();
    claim(
        &store,
        "host/birch",
        "fleet.member-removed",
        None,
        json!({"member_key":key.public(),"high_water":high_water,"reason":"Replace incarnation"}),
    );
    store.replication_snapshot().unwrap();
    let replacement = MemberKey::generate().unwrap().0;
    claim(
        &store,
        "host/birch",
        "fleet.member-admitted",
        None,
        json!({"fleet_id":FLEET,"member_key":replacement.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}),
    );
    store.replication_snapshot().unwrap();
    let source = "version 2\nagent \"new\" { command \"true\"; declared-lane \"check\"; }\n";
    assert_eq!(
        publish(&store, source, "replacement-missing")
            .unwrap_err()
            .code,
        ERROR
    );
}

#[test]
fn conflicted_membership_refuses_declared_admission() {
    let (store, anchor) = enabled();
    let (peer, _) = peer(&store, &anchor);
    started(&peer, json!({"declared_admission":1}));
    sync(&peer, &store);
    let conflicting = MemberKey::generate().unwrap().0;
    claim(
        &store,
        "host/birch",
        "fleet.member-admitted",
        None,
        json!({"fleet_id":FLEET,"member_key":conflicting.public(),"via":"invite","sponsor":"host/alder","mode":"listening"}),
    );
    store.replication_snapshot().unwrap();
    let source = "version 2\nagent \"new\" { command \"true\"; declared-priority \"normal\"; }\n";
    assert_eq!(
        publish(&store, source, "conflicted").unwrap_err().code,
        ERROR
    );
}

#[test]
fn unrelated_json_keys_and_names_do_not_enable_the_barrier() {
    let store = Store::open_memory("alder").unwrap();
    let custom = json!({
        "declared_priority":"normal", "declared_lane":"check", "declared_budgets":{},
        "children":[{"name":"declared-priority"}],
        "declarations_kdl":"agent \"not-a-declaration\" { declared-budgets {} }"
    });
    let desired = json!({"fields":{"body":{
        "member":{"name":"declared-priority", "env":custom, "tags":custom},
        "desired":{"name":"agent","properties":custom,
            "children":[{"name":"env","properties":custom}]}
    }}});
    validate_claim(&store.readers.get(), "alder", "intent.desired", &desired).unwrap();
    let mission = json!({"fields":{"body":{
        "id":"plain", "tags":custom, "inputs":custom,
        "steps":{"work":{"env":custom,"products":[custom]}}
    }}});
    validate_claim(&store.readers.get(), "alder", "mission.published", &mission).unwrap();
}

#[test]
fn memberless_agent_children_still_require_the_barrier() {
    let store = Store::open_memory("alder").unwrap();
    for child in [
        "declared-priority \"normal\"",
        "declared-lane \"check\"",
        "declared-budgets {}",
    ] {
        let source = format!(
            "version 2\nagent \"example/new\" {{ {child}; exec \"work\" {{ command \"true\"; }}; }}\n"
        );
        let intent = crate::graph::parse_intent(&source, "alder").unwrap();
        let desired = &intent.subjects["agent/example/new"];
        assert!(desired.member.is_none());
        let body = json!({"fields":{"body":desired}});
        assert_eq!(
            validate_claim(&store.readers.get(), "alder", "intent.desired", &body)
                .unwrap_err()
                .code,
            ERROR
        );
        assert_eq!(
            validate_publication(&store.readers.get(), "alder", &intent)
                .unwrap_err()
                .code,
            ERROR
        );
    }
}

#[test]
fn attention_capability_is_additive_and_anchored() {
    let (store, anchor) = enabled();
    started(&store, json!({"declared_admission":1,"declared_attention":1}));
    let (peer, _) = peer(&store, &anchor);
    started(&peer, json!({"declared_admission":1}));
    sync(&peer, &store);
    publish(&store, "version 2\nmission \"memory\" state=\"ready\" { goal \"Work.\"; declared-budgets { memory-max \"8G\"; } }\n", "memory").unwrap();
    let attention = "version 2\nmission \"attention\" state=\"ready\" { goal \"Work.\"; declared-budgets { attention \"person/reviewer\" asks=3; } }\n";
    let before = store.index().unwrap();
    let error = publish(&store, attention, "attention").unwrap_err();
    assert_eq!(error.code, ERROR);
    assert!(error.message.contains("host/birch"));
    assert!(error.message.contains("features.declared_attention=1"));
    assert_eq!(store.index().unwrap(), before);
    started(&peer, json!({"declared_admission":1,"declared_attention":1}));
    assert_eq!(publish(&store, attention, "unanchored").unwrap_err().code, ERROR);
    sync(&peer, &store);
    publish(&store, attention, "supported").unwrap();
}
