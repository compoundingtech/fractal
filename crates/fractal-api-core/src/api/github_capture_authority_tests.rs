use super::*;

const OPERATOR: &str = "person/operator";
const SUBJECT: &str = "github-source/public-example";

fn source() -> Source {
    Source { host: "github.com".into(), id: "public-example".into(), generation: 1 }
}

fn root_grant() -> AuthorityGrant {
    AuthorityGrant { version: capture::VERSION, source: source(), member: "host-b".into(), epoch: 1,
        predecessor_grant: None, final_predecessor_commit: None, fencing_receipt: None }
}

fn fixture() -> Store {
    let key = Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0);
    let store = Store::open_memory("host-b").unwrap();
    let fleet = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    store.bind_fleet(fleet).unwrap();
    store.pin_fleet_anchor(key.public()).unwrap();
    store.set_member_key(Some(Arc::clone(&key))).unwrap();
    store.append_claim(&ClaimInput { subject: "host/host-b".into(), kind: "fleet.member-admitted".into(), actor: None,
        fields: serde_json::from_value(json!({"fleet_id":fleet,"member_key":key.public(),"via":"anchor","mode":"listening"})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: None }).unwrap();
    store.seal_local_batches().unwrap();
    store.judge_claims(true).unwrap();
    store.ensure_principal_key(OPERATOR).unwrap();
    store
}

fn append(store: &Store, grant: &AuthorityGrant) -> Result<ClaimRecord, FractalError> {
    append_grant(store, SUBJECT.into(), OPERATOR.into(), grant)
}

fn raw<T: Serialize>(store: &Store, subject: &str, kind: &str, record: &T) -> ClaimRecord {
    store.append_claim(&ClaimInput { subject: subject.into(), kind: kind.into(), actor: None,
        fields: BTreeMap::from([("record".into(), serde_json::to_value(record).unwrap())]),
        evidence: vec![], expected_subject: None, idempotency_key: None }).unwrap()
}

fn fenced_successor(store: &Store, prior: &ClaimRecord, final_commit: Option<String>) -> AuthorityGrant {
    let receipt = FenceReceipt { version: capture::VERSION, source: source(), member: "host-b".into(), epoch: 1,
        authority_grant: prior.id.clone(), final_commit: final_commit.clone(), stopped_at_ms: 100 };
    let claim = raw(store, SUBJECT, capture::FENCED_KIND, &receipt);
    AuthorityGrant { version: capture::VERSION, source: source(), member: "host-b".into(), epoch: 2,
        predecessor_grant: Some(prior.id.clone()), final_predecessor_commit: final_commit, fencing_receipt: Some(claim.id) }
}

#[test]
fn root_is_idempotent_and_does_not_allow_another_member_or_epoch() {
    let store = fixture();
    let grant = root_grant();
    let first = append(&store, &grant).unwrap();
    let second = append(&store, &grant).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.store_index, second.store_index);
    let mut changed = grant.clone();
    changed.member = "unadmitted".into();
    assert!(append(&store, &changed).is_err());
    let mut next = grant;
    next.epoch = 2;
    assert!(append(&store, &next).is_err());
}

#[test]
fn root_rejects_unadmitted_member_and_invalid_source_before_append() {
    let store = fixture();
    let mut grant = root_grant();
    grant.member = "not-a-member".into();
    assert!(append(&store, &grant).is_err());
    grant = root_grant();
    grant.source.generation = 0;
    assert!(append(&store, &grant).is_err());
    assert!(store.latest_claim(SUBJECT, Some(capture::AUTHORITY_KIND)).unwrap().is_none());
}

#[test]
fn successor_requires_receipt_and_exact_stopped_frontier_then_replays() {
    let store = fixture();
    let prior = append(&store, &root_grant()).unwrap();
    let mut next = fenced_successor(&store, &prior, None);
    let mut absent = next.clone();
    absent.fencing_receipt = Some("missing-receipt".into());
    assert!(append(&store, &absent).is_err());
    next.final_predecessor_commit = Some("not-the-stopped-frontier".into());
    assert!(append(&store, &next).is_err());
    next.final_predecessor_commit = None;
    let first = append(&store, &next).unwrap();
    assert_eq!(append(&store, &next).unwrap().id, first.id);
    assert!(append(&store, &root_grant()).is_ok(), "an exact old operation still replays");
}

#[test]
fn successor_rejects_wrong_source_epoch_and_predecessor() {
    let store = fixture();
    let prior = append(&store, &root_grant()).unwrap();
    let next = fenced_successor(&store, &prior, None);
    let mut changed = next.clone();
    changed.source.generation = 2;
    assert!(append(&store, &changed).is_err());
    changed = next.clone();
    changed.epoch = 3;
    assert!(append(&store, &changed).is_err());
    changed = next;
    changed.predecessor_grant = Some("not-the-predecessor".into());
    assert!(append(&store, &changed).is_err());
}

#[test]
fn successor_checks_frontier_capture_authority() {
    let store = fixture();
    let prior = append(&store, &root_grant()).unwrap();
    let mut commit = CaptureCommit { version: capture::VERSION, source: source(), epoch: 1,
        authority_grant: "another-grant".into(), sequence: 1, predecessor: None,
        facts: vec![], checkpoint: None, coverage: vec![], observed_at_ms: 100 };
    let claim = raw(&store, &commit.subject().unwrap(), capture::COMMIT_KIND, &commit);
    let next = fenced_successor(&store, &prior, Some(claim.id));
    assert!(append(&store, &next).is_err());
    commit.authority_grant = prior.id.clone();
    let claim = raw(&store, &commit.subject().unwrap(), capture::COMMIT_KIND, &commit);
    let next = fenced_successor(&store, &prior, Some(claim.id));
    assert!(append(&store, &next).is_ok());
}

#[test]
fn receipt_must_be_from_the_granted_member() {
    let store = fixture();
    let prior = append(&store, &root_grant()).unwrap();
    let receipt = FenceReceipt { version: capture::VERSION, source: source(), member: "another-member".into(), epoch: 1,
        authority_grant: prior.id.clone(), final_commit: None, stopped_at_ms: 100 };
    let receipt = raw(&store, SUBJECT, capture::FENCED_KIND, &receipt);
    let next = AuthorityGrant { version: capture::VERSION, source: source(), member: "host-b".into(), epoch: 2,
        predecessor_grant: Some(prior.id), final_predecessor_commit: None, fencing_receipt: Some(receipt.id) };
    assert!(append(&store, &next).is_err());
}

#[test]
fn operators_are_not_inferred_from_bound_agent_or_body() {
    let directory = tempfile::tempdir().unwrap();
    let mut state = super::super::tests::state(directory.path());
    assert!(operator(&state, None, false, OPERATOR).is_err());
    state.owning_person = Some(OPERATOR.into());
    assert!(operator(&state, None, false, OPERATOR).is_ok());
    assert!(operator(&state, None, true, OPERATOR).is_err(), "client sessions cannot act as local operators");
    assert!(operator(&state, None, false, "person/other").is_err());
    for bound in [BoundActor::Agent("agent/example".into()), BoundActor::Run("mission-run/example".into())] {
        assert!(operator(&state, Some(&bound), false, OPERATOR).is_err());
    }
    assert!(source_path("other", &source()).is_err());
}

fn declare(store: &Store, grant: &str, generation: u64) {
    let kdl = format!("version 2\ngithub-source \"public-example\" {{ host \"github.com\"; generation {generation}; authority-grant \"{grant}\"; repository \"example/public\"; enabled #true; checkpoint-every-ms 60000; replicate-private #false; }}");
    let intent = crate::graph::parse_intent(&kdl, store.origin()).unwrap();
    store.apply_internal(&intent, &format!("source-{generation}-{grant}")).unwrap();
}

#[test]
fn admission_checks_the_current_declaration_source_identity() {
    let store = fixture();
    declare(&store, "not-yet-admitted", 2);
    assert!(append(&store, &root_grant()).is_err());
    assert!(store.latest_claim(SUBJECT, Some(capture::AUTHORITY_KIND)).unwrap().is_none());
}

#[test]
fn disabled_source_can_publish_before_root_admission_then_enable_exact_grant() {
    let store = fixture();
    let disabled = r#"version 2
github-source "public-example" {
    host "github.com"
    generation 1
    repository "example/public"
    enabled #false
    checkpoint-every-ms 60000
    replicate-private #false
}
"#;
    let intent = crate::graph::parse_intent(disabled, store.origin()).unwrap();
    store.apply_internal(&intent, "bootstrap-disabled-source").unwrap();
    let definitions = store.github_source_definitions().unwrap();
    assert_eq!(definitions.len(), 1);
    assert!(!definitions[0].enabled);
    assert_eq!(definitions[0].authority_grant, None);
    assert!(store.latest_claim(SUBJECT, Some(capture::AUTHORITY_KIND)).unwrap().is_none());
    let grant = append(&store, &root_grant()).unwrap();
    assert_eq!(append(&store, &root_grant()).unwrap().id, grant.id);
    // Root admission does not silently activate or rewrite the authored declaration.
    assert_eq!(store.github_source_definitions().unwrap(), definitions);
    declare(&store, &grant.id, 1);
    let enabled = store.github_source_definitions().unwrap();
    assert!(enabled[0].enabled);
    assert_eq!(enabled[0].authority_grant.as_deref(), Some(grant.id.as_str()));
}

#[tokio::test]
async fn fence_rejects_stale_grant_or_generation_without_writing_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let mut state = super::super::tests::state(directory.path());
    state.owning_person = Some(OPERATOR.into());
    state.store = Arc::new(fixture());
    let prior = append(&state.store, &root_grant()).unwrap();
    declare(&state.store, &prior.id, 1);
    let mut request = FenceRequest { actor: OPERATOR.into(), source: source(), authority_grant: "stale-grant".into() };
    assert!(fence(State(state.clone()), None, None, AxumPath(source().id), Json(request.clone())).await.is_err());
    request.authority_grant = prior.id;
    request.source.generation = 2;
    assert!(fence(State(state.clone()), None, None, AxumPath(source().id), Json(request)).await.is_err());
    assert!(state.store.latest_claim(SUBJECT, Some(capture::FENCED_KIND)).unwrap().is_none());
}
