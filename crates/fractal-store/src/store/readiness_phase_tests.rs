use super::*;

const CASE: &str = "slo-objective/readiness-case/executor/fractal-api-readiness/member/120000";
const OWNER: &str = "agent/factory/owner";

fn is_phase_case(
    episode: &str,
    origin: &str,
    actor: &str,
    condition: &str,
    reviewer: &str,
) -> bool {
    readiness_phase_delivery(
        "operational.failure",
        Some(actor),
        "daemon/executor",
        origin,
        Some(condition),
        Some(reviewer),
        episode,
    )
}

#[test]
fn routing_requires_the_exact_system_only_case() {
    let is_case = |episode| {
        is_phase_case(
            episode,
            "executor",
            "daemon/runtime",
            API_READINESS_SLO_MISSED,
            OWNER,
        )
    };
    assert!(is_case(CASE));
    for episode in [
        "slo-objective/readiness-case-extra/executor/fractal-api-readiness/member/120000",
        "slo-objective/readiness-case/executor/fractal-api-readiness/member/120001",
        "slo-objective/readiness-case/executor/fractal-api-readiness/member/0120000",
        "slo-objective/fractal-api-readiness/member",
        "ordinary-fault",
    ] {
        assert!(!is_case(episode), "{episode}");
    }
    assert!(!is_phase_case(
        CASE,
        "other",
        "daemon/runtime",
        API_READINESS_SLO_MISSED,
        OWNER
    ));
    assert!(!is_phase_case(
        CASE,
        "executor",
        OWNER,
        API_READINESS_SLO_MISSED,
        OWNER
    ));
    assert!(!is_phase_case(
        CASE,
        "executor",
        "daemon/runtime",
        "observed",
        OWNER
    ));
    assert!(!is_phase_case(
        CASE,
        "executor",
        "daemon/runtime",
        API_READINESS_SLO_MISSED,
        "person/operator"
    ));
}

#[test]
fn escalation_cannot_change_the_opening_owner_or_link_a_different_case() {
    let opening: ClaimRecord = serde_json::from_value(json!({
        "id": "a".repeat(64), "store_index": 1, "batch_id": "fixture",
        "kind": "operational.failure", "origin": "executor",
        "actor": "daemon/runtime", "subject": "daemon/executor",
        "body": {"fields": {"episode": CASE, "condition": API_READINESS_SLO_MISSED, "reviewer": OWNER}, "evidence": []},
        "predecessors": [], "accepted_at_unix_ms": 0
    })).unwrap();
    assert!(valid_opening_claim_id(&opening.id));
    assert!(!valid_opening_claim_id("not-a-claim-id"));
    assert_eq!(
        linked_readiness_reviewer("executor", CASE, &opening),
        Some(OWNER)
    );
    assert_eq!(linked_readiness_reviewer("other", CASE, &opening), None);
    assert_eq!(
        linked_readiness_reviewer("executor", "ordinary-fault", &opening),
        None
    );
}

fn failure(store: &Store, episode: &str, reviewer: &str, evidence: Vec<String>) -> ClaimRecord {
    store
        .append_claim(&ClaimInput {
            subject: "daemon/executor".into(),
            kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("episode".into(), json!(episode)),
                ("condition".into(), json!(API_READINESS_SLO_MISSED)),
                ("reviewer".into(), json!(reviewer)),
                ("severity".into(), json!("warning")),
                ("title".into(), json!("Fixture fault")),
                ("reason".into(), json!("Fixture evidence")),
                ("targets".into(), json!(["daemon/executor"])),
            ]),
            evidence,
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
}

#[test]
fn late_contradictory_opening_never_enters_generic_delivery() {
    use smallclaims::replication::ReplicationInventory;

    const FLEET: &str = "fleet/readiness-fixture";
    const SUBSTITUTE: &str = "agent/factory/substitute";
    let source = Store::open_memory("executor").unwrap();
    let receiver = Store::open_memory("receiver").unwrap();
    for store in [&source, &receiver] {
        store.bind_fleet(FLEET).unwrap();
    }
    let opening = failure(&source, CASE, OWNER, Vec::new());
    let escalation = failure(&source, CASE, SUBSTITUTE, vec![opening.id.clone()]);
    let receive = |latest| {
        let mut batch = source
            .export_replication_exchange(FLEET, &ReplicationInventory::default())
            .unwrap();
        batch.envelopes.sort_by_key(|envelope| envelope.sequence);
        assert_eq!(batch.envelopes.len(), 2);
        batch.envelopes = vec![batch.envelopes.remove(usize::from(latest))];
        receiver
            .receive_replication_exchange("executor", FLEET, &batch)
            .unwrap();
        receiver.validate_replication_backlog().unwrap();
        receiver.apply_replication_repairs().unwrap();
        receiver.project_replication_backlog().unwrap();
    };
    let as_of = escalation.accepted_at_unix_ms + 10;
    let visible = |diagnostic: &str, expected_person: &str| {
        let faults = receiver.fault_snapshot(as_of).unwrap();
        let fault = faults
            .iter()
            .find(|fault| fault.item.episode == CASE)
            .unwrap();
        assert!(fault.item.detail.contains(diagnostic), "{:?}", fault.item);
        assert_eq!(fault.item.person, expected_person);
        assert!(
            !receiver
                .fault_delivery_snapshot(as_of)
                .unwrap()
                .iter()
                .any(|fault| fault.item.episode == CASE)
        );
    };
    receive(true);
    assert!(receiver.claim_by_id(&opening.id).unwrap().is_none());
    visible("awaits the linked opening claim", "person/operator");
    receive(false);
    visible("blocked by a contradictory opening claim", OWNER);

    let ordinary = failure(
        &receiver,
        "slo-objective/fractal-api-readiness/member",
        SUBSTITUTE,
        Vec::new(),
    );
    assert!(
        receiver
            .fault_delivery_snapshot(as_of.max(ordinary.accepted_at_unix_ms))
            .unwrap()
            .iter()
            .any(|fault| fault.item.episode == ordinary.id)
    );
}
