//! Crash cuts use durable admission and real native messages, never timers or mock senders.
use super::*;
use crate::store::effects::{EffectAdmission, EffectRecord, native_message_key};
use crate::model::EffectOwner;

const FLEET: &str = "native-effect-recovery-fixture";
const INCARNATION: &str = "example-worker:one";
const RECOVERING_WORKER: &str = "agent/example/replacement";

struct Fixture {
    root: tempfile::TempDir,
    state: AppState,
    request: MessageSendRequest,
}

impl Fixture {
    fn new() -> Self {
        Self::on_origin("origin")
    }

    fn on_origin(origin: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let state = persistent_state(root.path(), origin);
        state.store.bind_fleet(FLEET).unwrap();
        state.store.apply_internal(
            &parse_intent(r#"version 2
agent "example/worker" { workspace "/tmp"; harness "claude" {} }
agent "example/replacement" { workspace "/tmp"; harness "claude" {} }
agent "example/reader" { workspace "/tmp"; harness "claude" {} }
mission "example/report" state="ready" {
  goal "Report the result once."
  step "report" {
    available-to "agent/example/worker"
    available-to "agent/example/replacement"
    goal "Send the report."
  }
}
"#, "origin").unwrap(), "effect-fixture",
        ).unwrap();
        let run = state.store.create_mission_run(&MissionRunRequest {
            mission: "example/report".into(), revision: None, workspace: "/tmp".into(),
            requester: Some("person/example".into()), mode: Some("run".into()),
            inputs: BTreeMap::new(), idempotency_key: "example-run".into(),
        }).unwrap();
        let step = &run.steps[0].subject;
        state.store.set_step_state(step, "ready", None).unwrap();
        let claimed = state.store.work_action(step, "claim", &WorkRequest {
            wait_ms: None, actor: Some("agent/example/worker".into()),
            incarnation: Some(INCARNATION.into()), summary: None, reason: None,
            evidence: vec![], idempotency_key: "example-claim".into(),
        }).unwrap();
        let sender = claimed.claimant.unwrap();
        runtime(&state, &sender, INCARNATION);
        let request = MessageSendRequest {
            idempotency_key: "caller-key-is-not-the-effect-key".into(),
            from: sender, to: "agent/example/reader".into(), content: "The report is ready.".into(),
            title: Some("Example report".into()), in_reply_to: None, tags: vec![], attachments: vec![],
            owner: Some(EffectOwner {
                mission: run.mission.strip_prefix("mission/").unwrap().into(),
                key: "report-ready".into(), step: step.clone(), attempt: claimed.attempt,
                incarnation: INCARNATION.into(),
            }),
        };
        Self { root, state, request }
    }

    fn owner(&self) -> &EffectOwner { self.request.owner.as_ref().unwrap() }

    fn admit(&self) -> EffectRecord {
        let (_, sender) = receipt_recipient(&self.state.store, &self.request.from).unwrap();
        let (_, target) = receipt_recipient(&self.state.store, &self.request.to).unwrap();
        match self.state.store.admit_message_effect(self.owner(), &sender, &sender, &payload_digest(&self.request), &target).unwrap() {
            EffectAdmission::Originate(record) => record,
            EffectAdmission::Recover(_) | EffectAdmission::Blocked(_) => panic!("first local admission must originate"),
        }
    }

    fn restart(&mut self) {
        // Release the old Store before opening the same durable database again.
        let placeholder = super::tests::state(self.root.path());
        drop(std::mem::replace(&mut self.state, placeholder));
        self.state = persistent_state(self.root.path(), "origin");
    }

    fn replace_attempt(&mut self) {
        let sender = self.request.from.clone();
        self.replace_attempt_as(&sender, "example-worker:replacement");
    }

    fn replace_attempt_as(&mut self, actor: &str, incarnation: &str) -> String {
        let step = self.owner().step.clone();
        let previous_attempt = self.owner().attempt;
        self.state.store.set_step_state(&step, "failed", Some("interrupted worker")).unwrap();
        assert!(self.state.store.retry_step(&step, "replace interrupted worker", 0).unwrap());
        self.state.store.set_step_state(&step, "ready", None).unwrap();
        let (_, caller) = receipt_recipient(&self.state.store, actor).unwrap();
        runtime(&self.state, &caller, incarnation);
        let claimed = self.state.store.work_action(&step, "claim", &WorkRequest {
            wait_ms: None, actor: Some(caller.clone()),
            incarnation: Some(incarnation.into()), summary: None, reason: None,
            evidence: vec![], idempotency_key: format!("replacement-claim/{caller}/{previous_attempt}/{incarnation}"),
        }).unwrap();
        assert!(claimed.attempt > previous_attempt);
        assert_eq!(claimed.subject, step);
        assert_eq!(claimed.claimant.as_deref(), Some(caller.as_str()));
        let owner = self.request.owner.as_mut().unwrap();
        owner.attempt = claimed.attempt;
        owner.incarnation = incarnation.into();
        caller
    }

    fn accept(&self) -> Result<MessageSendReceipt, ApiError> {
        verified_send(&self.state, self.request.clone())
    }

    fn seed_message(&self) -> MessageSendReceipt {
        let mut request = self.request.clone();
        request.idempotency_key = native_message_key(self.owner()).unwrap();
        request.owner = None;
        accept_message_receipt(&self.state, request, None, None).unwrap()
    }

    fn assert_parked(&self, effect: &EffectRecord) {
        let unresolved = self.state.store.claims_for(&effect.effect_id, Some("effect.unresolved")).unwrap();
        assert_eq!(unresolved.len(), 1);
        let step = self.state.store.claims_for(&self.owner().step, Some("step-run.state")).unwrap();
        assert!(step.iter().any(|claim| claim.body["fields"]["status"] == "blocked"
            && claim.body["fields"]["reason"] == "effect-unresolved"
            && claim.body["evidence"].as_array().unwrap().contains(&json!(effect.effect_id))));
        assert_eq!(self.state.store.step_run(&self.owner().step).unwrap().unwrap().status, "blocked");
    }
}

#[test]
fn daemon_capabilities_refuse_unknown_provider_class_and_target_before_admission() {
    use crate::store::effects::{EffectClass, EffectTarget};
    let fixture = Fixture::new();
    for (provider, class, target, code) in [
        ("sdk-provider", EffectClass::NativeCoordinated, "agent/example/reader", "effect-provider-unknown"),
        ("native-message", EffectClass::ProviderIdempotent, "agent/example/reader", "effect-class-mismatch"),
        ("native-message", EffectClass::NativeCoordinated, "not-a-recipient", "effect-target-mismatch"),
        ("git.branch-create", EffectClass::ProviderIdempotent, r#"{"repo":"example/repo"}"#, "effect-target-mismatch"),
    ] {
        let error = fixture.state.store.admit_effect(fixture.owner(), &fixture.request.from,
            &fixture.request.from, "digest", EffectTarget { provider, class, target }).unwrap_err();
        assert_eq!(error.code, code);
    }
    // Refused declarations cannot consume the key or widen native-message capability.
    fixture.admit();
}

fn persistent_state(root: &Path, origin: &str) -> AppState {
    let mut state = super::tests::state(root);
    state.store = Arc::new(Store::open(&root.join("graph.db"), origin).unwrap());
    state.node = origin.into();
    state
}

fn runtime(state: &AppState, sender: &str, incarnation: &str) {
    for (kind, fields) in [
        ("runtime.observed", BTreeMap::from([("status".into(), json!("running")), ("incarnation_id".into(), json!(incarnation))])),
        ("harness.observed", BTreeMap::from([("state".into(), json!("working")), ("driver".into(), json!("claude")), ("incarnation_id".into(), json!(incarnation))])),
    ] {
        state.store.append_claim(&ClaimInput {
            subject: sender.into(), kind: kind.into(), actor: Some(sender.into()), fields,
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
}

fn payload_digest(request: &MessageSendRequest) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(&(
        &request.from, &request.to, &request.content, &request.title,
        &request.in_reply_to, &request.tags, &request.attachments,
    )).unwrap()))
}

fn verified_send(state: &AppState, request: MessageSendRequest) -> Result<MessageSendReceipt, ApiError> {
    let sender = request.from.clone();
    verified_send_as(state, request, &sender)
}

fn verified_send_as(state: &AppState, request: MessageSendRequest, caller: &str) -> Result<MessageSendReceipt, ApiError> {
    accept_message_receipt_authorized(state, request, None, None, None, None, Some(caller))
}

fn assert_message_count(state: &AppState, expected: usize) {
    let messages = state.store.messages(None, false).unwrap();
    assert_eq!(messages.len(), expected);
    for message in messages {
        assert_eq!(state.store.claims_for(&message.subject, Some("message.sent")).unwrap().len(), 1,
            "a recovery must not originate another message claim");
    }
}

fn replicate(from: &AppState, to: &AppState) {
    // The same real envelope/drain sequence as tests/split_host_delivery.rs, without servers.
    for _ in 0..100 {
        let exchange = from.store.export_replication_exchange(FLEET, &to.store.replication_inventory().unwrap()).unwrap();
        if exchange.envelopes.is_empty() { return; }
        to.store.receive_replication_exchange(&from.node, FLEET, &exchange).unwrap();
        let admission = to.store.validate_replication_backlog().unwrap();
        assert_eq!((admission.invalid, admission.unknown), (0, 0));
        to.store.apply_replication_repairs().unwrap();
        to.store.project_replication_backlog().unwrap();
    }
    panic!("replication did not drain");
}

#[test]
fn intended_restart_recovers_an_existing_message_by_key_without_entering_send() {
    let mut fixture = Fixture::new();
    let effect = fixture.admit();
    let sent = fixture.seed_message();
    assert_message_count(&fixture.state, 1);
    fixture.restart();
    let recovered = fixture.accept().unwrap();
    assert!(recovered.already_sent);
    assert_eq!(recovered.message.subject, sent.message.subject);
    assert_eq!(recovered.idempotency_key, native_message_key(fixture.owner()).unwrap());
    assert!(fixture.state.store.claims_for(&effect.effect_id, Some("effect.in-flight")).unwrap().is_empty());
    let confirmed = fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap();
    assert_eq!(confirmed.provider_ref.as_deref(), Some(sent.message.subject.as_str()));
    assert_eq!(confirmed.evidence.len(), 1);
    assert_message_count(&fixture.state, 1);
}

#[test]
fn accepted_before_response_restart_recovers_and_confirms_the_same_message() {
    let mut fixture = Fixture::new();
    let effect = fixture.admit();
    fixture.state.store.effect_in_flight(&effect).unwrap();
    let sent = fixture.seed_message();
    // Cut after the provider commit, before effect confirmation or the caller's response.
    assert!(fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap().provider_ref.is_none());
    fixture.restart();
    fixture.replace_attempt();
    for _ in 0..2 {
        let recovered = fixture.accept().unwrap();
        assert!(recovered.already_sent);
        assert_eq!(recovered.message.subject, sent.message.subject);
    }
    assert_eq!(fixture.state.store.claims_for(&effect.effect_id, Some("effect.in-flight")).unwrap().len(), 1);
    assert_eq!(fixture.state.store.claims_for(&effect.effect_id, Some("effect.confirmed")).unwrap().len(), 1);
    assert_message_count(&fixture.state, 1);
}

#[test]
fn different_worker_recovers_a_confirmed_effect_without_originating_another_message() {
    let mut fixture = Fixture::new();
    let sent = fixture.accept().unwrap();
    let original = fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap();
    let original_payload = payload_digest(&fixture.request);
    fixture.restart();
    let caller = fixture.replace_attempt_as(RECOVERING_WORKER, "replacement-worker:one");
    assert_ne!(caller, fixture.request.from);
    assert_eq!(payload_digest(&fixture.request), original_payload);
    for _ in 0..2 {
        let recovered = verified_send_as(&fixture.state, fixture.request.clone(), &caller).unwrap();
        assert!(recovered.already_sent);
        assert_eq!(recovered.message.subject, sent.message.subject);
        assert_eq!(recovered.message.from, fixture.request.from);
    }
    let confirmed = fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap();
    assert_eq!(confirmed.recovering_actor.as_deref(), Some(caller.as_str()));
    assert_eq!(confirmed.lease_owner, original.lease_owner);
    assert_eq!(confirmed.lease_incarnation, original.lease_incarnation);
    assert_eq!(confirmed.bound_step_run, original.bound_step_run);
    assert_eq!(confirmed.bound_attempt, original.bound_attempt);
    assert_eq!(confirmed.payload_digest, original.payload_digest);
    assert_eq!(confirmed.evidence, original.evidence);
    assert_eq!(confirmed.provider_ref.as_deref(), Some(sent.message.subject.as_str()));
    assert_eq!(fixture.state.store.claims_for(&original.effect_id, Some("effect.confirmed")).unwrap().len(), 2);
    assert_eq!(fixture.state.store.claims_for(&original.effect_id, Some("effect.in-flight")).unwrap().len(), 1);
    assert_message_count(&fixture.state, 1);
}

#[test]
fn different_host_and_worker_recovers_a_long_message_with_its_original_body_origin() {
    let mut original = Fixture::new();
    original.request.content = "x".repeat(message_body::INLINE_MAX_BYTES + 1024);
    let effect = original.admit();
    original.state.store.effect_in_flight(&effect).unwrap();
    let sent = original.seed_message();
    assert_eq!(sent.message.body_origin.as_deref(), Some("host/origin"));
    let committed = original.state.store.operation_claim(&sent.idempotency_key).unwrap().unwrap();
    let peer_root = tempfile::tempdir().unwrap();
    let peer = persistent_state(peer_root.path(), "peer");
    peer.store.bind_fleet(FLEET).unwrap();
    replicate(&original.state, &peer);
    let mut recovering = Fixture {
        root: peer_root, state: peer, request: original.request.clone(),
    };
    let caller = recovering.replace_attempt_as(RECOVERING_WORKER, "replacement-worker:peer");
    assert_ne!(client_host_id(&recovering.state.node), "host/origin");
    for _ in 0..2 {
        let recovered = verified_send_as(&recovering.state, recovering.request.clone(), &caller).unwrap();
        assert!(recovered.already_sent);
        assert_eq!(recovered.message.subject, sent.message.subject);
        assert_eq!(recovered.message.body_origin, sent.message.body_origin);
        assert_eq!(recovered.message.body_ref, sent.message.body_ref);
        assert_eq!(recovered.message.body_bytes, sent.message.body_bytes);
    }
    let confirmed = recovering.state.store.resolve_effect_by_key(recovering.owner()).unwrap().unwrap();
    assert_eq!(confirmed.provider_ref.as_deref(), Some(sent.message.subject.as_str()));
    assert_eq!(confirmed.recovering_actor.as_deref(), Some(caller.as_str()));
    assert!(confirmed.error.is_none());
    assert!(!recovering.state.store.claims_for(&effect.effect_id, Some("effect.confirmed")).unwrap().is_empty());
    assert!(recovering.state.store.claims_for(&effect.effect_id, Some("effect.unresolved")).unwrap().is_empty());
    assert_eq!(recovering.state.store.claims_for(&effect.effect_id, Some("effect.in-flight")).unwrap().len(), 1);
    assert_ne!(recovering.state.store.step_run(&recovering.owner().step).unwrap().unwrap().status, "blocked");
    let recovered_claim = recovering.state.store.operation_claim(&sent.idempotency_key).unwrap().unwrap();
    assert_eq!(recovered_claim.id, committed.id);
    assert_eq!(recovered_claim.body["fields"]["attachments"], committed.body["fields"]["attachments"]);
    assert_message_count(&original.state, 1);
    assert_message_count(&recovering.state, 1);
}

#[test]
fn long_message_recovery_refuses_different_full_text_despite_an_identical_preview() {
    for change_size in [false, true] {
        let mut fixture = Fixture::new();
        fixture.request.content = "x".repeat(message_body::INLINE_MAX_BYTES + 1024);
        let effect = fixture.admit();
        let mut conflicting = fixture.request.clone();
        conflicting.owner = None;
        conflicting.idempotency_key = native_message_key(fixture.owner()).unwrap();
        if !change_size {
            conflicting.content.pop();
        }
        conflicting.content.push('y');
        assert_eq!(message_body::preview(&conflicting.content), message_body::preview(&fixture.request.content));
        assert_eq!(conflicting.content.len() != fixture.request.content.len(), change_size);
        accept_message_receipt(&fixture.state, conflicting, None, None).unwrap();
        assert_eq!(fixture.accept().unwrap_err().code, "idempotency-mismatch");
        fixture.assert_parked(&effect);
        assert!(fixture.state.store.claims_for(&effect.effect_id, Some("effect.confirmed")).unwrap().is_empty());
        assert_message_count(&fixture.state, 1);
    }
}

#[test]
fn different_worker_parks_an_undelivered_effect_instead_of_sending() {
    for in_flight in [false, true] {
        let mut fixture = Fixture::new();
        let original = fixture.admit();
        if in_flight { fixture.state.store.effect_in_flight(&original).unwrap(); }
        fixture.restart();
        let caller = fixture.replace_attempt_as(RECOVERING_WORKER, "replacement-worker:one");
        assert_eq!(verified_send_as(&fixture.state, fixture.request.clone(), &caller).unwrap_err().code,
            "effect-unresolved");
        fixture.assert_parked(&original);
        let unresolved = fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap();
        assert_eq!(unresolved.recovering_actor.as_deref(), Some(caller.as_str()));
        assert_eq!(unresolved.lease_owner, original.lease_owner);
        assert_eq!(unresolved.bound_step_run, original.bound_step_run);
        assert_eq!(unresolved.bound_attempt, original.bound_attempt);
        assert_eq!(unresolved.payload_digest, original.payload_digest);
        assert!(fixture.state.store.claims_for(&original.effect_id, Some("effect.confirmed")).unwrap().is_empty());
        assert_eq!(fixture.state.store.claims_for(&original.effect_id, Some("effect.in-flight")).unwrap().len(),
            usize::from(in_flight));
        assert!(verified_send_as(&fixture.state, fixture.request.clone(), &caller).is_err());
        assert_message_count(&fixture.state, 0);
    }
}

#[test]
fn different_worker_cannot_recover_with_a_stale_replacement_attempt() {
    let mut fixture = Fixture::new();
    let original = fixture.admit();
    fixture.seed_message();
    let caller = fixture.replace_attempt_as(RECOVERING_WORKER, "replacement-worker:one");
    let stale_request = fixture.request.clone();
    // Keep B's incarnation live: refusal must fence the old attempt, not a dead worker.
    fixture.replace_attempt_as(RECOVERING_WORKER, "replacement-worker:one");
    assert_eq!(verified_send_as(&fixture.state, stale_request, &caller).unwrap_err().code,
        "stale-work-lease");
    assert_eq!(fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap(), original);
    assert!(fixture.state.store.claims_for(&original.effect_id, Some("effect.confirmed")).unwrap().is_empty());
    assert!(fixture.state.store.claims_for(&original.effect_id, Some("effect.unresolved")).unwrap().is_empty());
    assert_message_count(&fixture.state, 1);
}

#[test]
fn different_worker_changing_the_original_payload_parks_without_a_second_send() {
    for field in ["from", "content"] {
        let mut fixture = Fixture::new();
        let original = fixture.admit();
        fixture.seed_message();
        let caller = fixture.replace_attempt_as(RECOVERING_WORKER, "replacement-worker:one");
        let mut changed = fixture.request.clone();
        match field {
            "from" => changed.from.clone_from(&caller),
            "content" => changed.content.push_str(" Changed by the replacement worker."),
            _ => unreachable!(),
        }
        assert_eq!(verified_send_as(&fixture.state, changed, &caller).unwrap_err().code,
            "idempotency-mismatch", "{field}");
        fixture.assert_parked(&original);
        let unresolved = fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap();
        assert_eq!(unresolved.recovering_actor.as_deref(), Some(caller.as_str()));
        assert_eq!(unresolved.lease_owner, original.lease_owner);
        assert_eq!(unresolved.payload_digest, original.payload_digest);
        assert!(fixture.state.store.claims_for(&original.effect_id, Some("effect.in-flight")).unwrap().is_empty());
        assert!(fixture.state.store.claims_for(&original.effect_id, Some("effect.confirmed")).unwrap().is_empty());
        assert_message_count(&fixture.state, 1);
    }
}

#[test]
fn different_worker_cannot_originate_an_effect_as_the_previous_sender() {
    let mut fixture = Fixture::new();
    let caller = fixture.replace_attempt_as(RECOVERING_WORKER, "replacement-worker:one");
    assert_ne!(caller, fixture.request.from);
    assert_eq!(verified_send_as(&fixture.state, fixture.request.clone(), &caller).unwrap_err().code,
        "effect-caller-refused");
    assert!(fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().is_none());
    assert_message_count(&fixture.state, 0);
}

#[test]
fn changed_payload_parks_the_effect_and_step_without_a_second_send() {
    // Exercise changes beyond content against the same immutable effect key.
    for field in ["to", "content", "title", "in_reply_to", "tags"] {
        let fixture = Fixture::new();
        let effect = fixture.admit();
        let sent = fixture.seed_message();
        let mut changed = fixture.request.clone();
        match field {
            "to" => changed.to = "agent/example/other-reader".into(),
            "content" => changed.content.push_str(" Changed."),
            "title" => changed.title = Some("Changed title".into()),
            "in_reply_to" => changed.in_reply_to = Some(sent.message.subject.clone()),
            "tags" => changed.tags.push("example-tag".into()),
            _ => unreachable!(),
        }
        let error = verified_send(&fixture.state, changed).unwrap_err();
        assert_eq!(error.code, "idempotency-mismatch", "{field}");
        fixture.assert_parked(&effect);
        assert!(fixture.state.store.claims_for(&effect.effect_id, Some("effect.in-flight")).unwrap().is_empty());
        assert_message_count(&fixture.state, 1);
    }
}

#[test]
fn recovery_refuses_a_different_payload_already_using_the_message_key() {
    let fixture = Fixture::new();
    let effect = fixture.admit();
    let mut conflicting = fixture.request.clone();
    conflicting.owner = None;
    conflicting.idempotency_key = native_message_key(fixture.owner()).unwrap();
    conflicting.content = "A different immutable message.".into();
    accept_message_receipt(&fixture.state, conflicting, None, None).unwrap();
    assert_eq!(fixture.accept().unwrap_err().code, "idempotency-mismatch");
    fixture.assert_parked(&effect);
    assert_message_count(&fixture.state, 1);
}

#[test]
fn confirmation_requires_the_exact_native_message_operation() {
    let fixture = Fixture::new();
    let effect = fixture.admit();
    let mut unrelated = fixture.request.clone();
    unrelated.owner = None;
    unrelated.idempotency_key = "unrelated-native-message".into();
    accept_message_receipt(&fixture.state, unrelated, None, None).unwrap();
    let message = fixture.state.store.operation_claim("unrelated-native-message").unwrap().unwrap();
    assert_eq!(fixture.state.store.confirm_message_effect(&effect, &message, &fixture.request.from).unwrap_err().code,
        "effect-evidence-mismatch");
    assert!(fixture.state.store.claims_for(&effect.effect_id, Some("effect.confirmed")).unwrap().is_empty());
    assert_message_count(&fixture.state, 1);
}

#[test]
fn restarted_worker_cannot_admit_with_the_old_incarnation() {
    let fixture = Fixture::new();
    runtime(&fixture.state, &fixture.request.from, "example-worker:two");
    assert_eq!(fixture.accept().unwrap_err().code, "stale-work-lease");
    assert!(fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().is_none());
    assert_message_count(&fixture.state, 0);
}

#[test]
fn unattested_effect_caller_is_refused_before_admission() {
    let fixture = Fixture::new();
    assert_eq!(accept_message_receipt(&fixture.state, fixture.request.clone(), None, None).unwrap_err().code,
        "effect-caller-refused");
    assert!(fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().is_none());
    assert_message_count(&fixture.state, 0);
}

#[tokio::test]
async fn native_handler_refuses_an_unattested_caller_before_any_effect_is_admitted() {
    let fixture = Fixture::new();
    let error = messages::send_message(
        State(fixture.state.clone()), None, None, None, None, Json(fixture.request.clone()),
    ).await.unwrap_err();
    assert_eq!(error.code, "effect-caller-refused");
    assert!(fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().is_none());
    assert_message_count(&fixture.state, 0);
}

#[test]
fn intended_restart_without_a_local_receipt_parks_instead_of_sending() {
    for in_flight in [false, true] {
        let mut fixture = Fixture::new();
        let effect = fixture.admit();
        if in_flight { fixture.state.store.effect_in_flight(&effect).unwrap(); }
        fixture.restart();
        assert_eq!(fixture.accept().unwrap_err().code, "effect-unresolved");
        fixture.assert_parked(&effect);
        assert_message_count(&fixture.state, 0);
        // A repeated request remains fenced even though no message is visible locally.
        assert!(fixture.accept().is_err());
        assert_message_count(&fixture.state, 0);
    }
}

#[test]
fn split_host_first_send_converges_with_a_durable_peer_fence_and_never_two_messages() {
    let fixture = Fixture::new();
    let peer_root = tempfile::tempdir().unwrap();
    let peer = persistent_state(peer_root.path(), "peer");
    peer.store.bind_fleet(FLEET).unwrap();
    replicate(&fixture.state, &peer);
    let barrier = std::sync::Barrier::new(2);
    let (origin_result, peer_result) = std::thread::scope(|scope| {
        let origin = scope.spawn(|| { barrier.wait(); fixture.accept() });
        let other = scope.spawn(|| { barrier.wait(); verified_send(&peer, fixture.request.clone()) });
        (origin.join().unwrap(), other.join().unwrap())
    });
    let sent = origin_result.unwrap();
    assert!(!sent.already_sent);
    assert_eq!(peer_result.unwrap_err().code, "effect-unresolved");
    assert_message_count(&fixture.state, 1);
    assert_message_count(&peer, 0);
    let parked = peer.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap();
    assert!(parked.error.is_some());
    assert_eq!(peer.store.claims_for(&parked.effect_id, Some("effect.unresolved")).unwrap().len(), 1);
    assert_eq!(peer.store.step_run(&fixture.owner().step).unwrap().unwrap().status, "blocked");
    replicate(&fixture.state, &peer);
    // Confirmed evidence arriving later must not erase the peer's unresolved fence.
    assert!(verified_send(&peer, fixture.request.clone()).is_err());
    assert!(peer.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap().error.is_some());
    assert_eq!(peer.store.messages(None, false).unwrap()[0].subject, sent.message.subject);
    replicate(&peer, &fixture.state);
    assert!(fixture.accept().is_err());
    fixture.assert_parked(&fixture.state.store.resolve_effect_by_key(fixture.owner()).unwrap().unwrap());
    replicate(&fixture.state, &peer);
    assert_message_count(&fixture.state, 1);
    assert_message_count(&peer, 1);
}

#[test]
fn visible_two_writer_admission_conflict_parks_before_either_host_sends() {
    for at_send_boundary in [false, true] {
        let first = Fixture::on_origin("first");
        let second = Fixture::on_origin("second");
        let left = first.admit();
        let right = second.admit();
        assert_eq!(left.operation_id, right.operation_id);
        // Both independently admitted before either intent was visible to the other.
        replicate(&first.state, &second.state);
        replicate(&second.state, &first.state);
        for (fixture, admitted) in [(&first, &left), (&second, &right)] {
            let key = crate::store::effects::effect_key(fixture.owner()).unwrap();
            let (writers, _) = fixture.state.store.operation_claims(&key).unwrap();
            assert_eq!(writers.len(), 2);
            let error = if at_send_boundary {
                fixture.state.store.effect_in_flight(admitted).unwrap_err().code.to_owned()
            } else {
                fixture.accept().unwrap_err().code
            };
            assert_eq!(error, "idempotency-conflict");
            fixture.assert_parked(admitted);
            assert!(fixture.state.store.effect_in_flight(admitted).is_err());
            assert_message_count(&fixture.state, 0);
        }
    }
}

#[test]
fn peer_replacement_lease_cannot_originate_an_absent_effect_from_a_foreign_creation_anchor() {
    let original = Fixture::new();
    let root = tempfile::tempdir().unwrap();
    let state = persistent_state(root.path(), "peer");
    state.store.bind_fleet(FLEET).unwrap();
    replicate(&original.state, &state);
    let mut peer = Fixture { root, state, request: original.request.clone() };
    let step = peer.owner().step.clone();
    let run = peer.state.store.step_run(&step).unwrap().unwrap().run;
    let anchor = peer.state.store.claims_for(&run, Some("mission-run.created")).unwrap();
    assert_eq!(anchor.len(), 1);
    assert_eq!(anchor[0].origin, "origin");
    assert!(peer.state.store.resolve_effect_by_key(peer.owner()).unwrap().is_none());
    assert_message_count(&peer.state, 0);

    let caller = peer.replace_attempt_as(RECOVERING_WORKER, "replacement-worker:peer");
    peer.request.from.clone_from(&caller);
    assert_eq!(peer.owner().step, step);
    let live_claims = peer.state.store.claims_for(&step, Some("work.claimed")).unwrap();
    assert!(live_claims.iter().any(|claim| claim.origin == "peer"
        && claim.actor.as_deref() == Some(caller.as_str())
        && claim.body["fields"]["attempt"] == json!(peer.owner().attempt)
        && claim.body["fields"]["claim_incarnation"] == peer.owner().incarnation));
    assert!(peer.state.store.resolve_effect_by_key(peer.owner()).unwrap().is_none());
    assert_eq!(verified_send_as(&peer.state, peer.request.clone(), &caller).unwrap_err().code,
        "effect-unresolved");
    let parked = peer.state.store.resolve_effect_by_key(peer.owner()).unwrap().unwrap();
    peer.assert_parked(&parked);
    assert_message_count(&peer.state, 0);
    assert!(peer.state.store.operation_claim(&native_message_key(peer.owner()).unwrap()).unwrap().is_none());
    assert!(verified_send_as(&peer.state, peer.request.clone(), &caller).is_err());
    assert_message_count(&peer.state, 0);
    let retained = peer.state.store.claims_for(&run, Some("mission-run.created")).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].id, anchor[0].id);
    assert_eq!(retained[0].origin, "origin");
    assert!(original.state.store.resolve_effect_by_key(original.owner()).unwrap().is_none());
    assert_message_count(&original.state, 0);
}

#[test]
fn unavailable_origin_and_missing_peer_receipt_never_authorize_a_resend() {
    let fixture = Fixture::new();
    let peer_root = tempfile::tempdir().unwrap();
    let peer = persistent_state(peer_root.path(), "peer");
    peer.store.bind_fleet(FLEET).unwrap();
    let effect = fixture.admit();
    fixture.state.store.effect_in_flight(&effect).unwrap();
    // Replicate the intent, but not the message the origin subsequently commits.
    replicate(&fixture.state, &peer);
    let sent = fixture.seed_message();
    assert_message_count(&fixture.state, 1);
    let request = fixture.request.clone();
    let owner = fixture.owner().clone();
    drop(fixture); // No originating store remains available to the recovering caller.
    assert_eq!(verified_send(&peer, request.clone()).unwrap_err().code, "effect-unresolved");
    assert!(verified_send(&peer, request).is_err());
    assert!(peer.store.operation_claim(&native_message_key(&owner).unwrap()).unwrap().is_none());
    assert!(peer.store.message(&sent.message.subject).unwrap().is_none());
    let parked = peer.store.resolve_effect_by_key(&owner).unwrap().unwrap();
    assert!(parked.error.is_some());
    assert_eq!(peer.store.claims_for(&effect.effect_id, Some("effect.unresolved")).unwrap().len(), 1);
    assert_message_count(&peer, 0);
}
