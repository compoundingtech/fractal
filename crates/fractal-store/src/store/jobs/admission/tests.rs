use super::*;
use serde_json::json;
use smallclaims::fleet::MemberKey;

fn fixture() -> (Snapshot, ClaimRecord, JobRecord, ClaimSignature, MemberKey) {
    let (key, _) = MemberKey::generate().unwrap();
    let binding = JobHostBinding {
        job_id: "job/fixture".into(), execution_host: "fixture-host".into(),
        authority_declaration: "claim/member".into(),
    };
    let record = JobRecord::Transition { binding, event: fractal_jobs::Event::Prepare };
    let claim = ClaimRecord {
        id: "claim/job".into(), store_index: 1, batch_id: "batch/fixture".into(),
        subject: "job/fixture".into(), kind: KIND.into(), origin: "fixture-host".into(),
        actor: Some("host/fixture-host".into()), operation_id: None, request_digest: None,
        body: json!({"fields":{"record":record}}), predecessors: vec![], accepted_at_unix_ms: 1,
    };
    let signature = sign(&key, &claim);
    let snapshot = Snapshot {
        local_origin: "fixture-host".into(),
        member: Some(Member { key: key.public().into(), declaration: "claim/member".into(), current: true }),
        existing_binding: None,
    };
    (snapshot, claim, record, signature, key)
}

fn sign(key: &MemberKey, claim: &ClaimRecord) -> ClaimSignature {
    ClaimSignature::sign(key,
        &content_digest(&claim.subject, &claim.kind, claim.actor.as_deref(), &claim.body),
        "host/fixture-host", None, vec![], 1)
}

fn refusal(result: Result<Disposition, Refusal>) -> Refusal { result.unwrap_err() }

#[test]
fn registry_is_disabled_for_every_kind_including_jobs() {
    for kind in [KIND, "attention.requested", "custom.fixture", "", "job.other"] {
        assert!(lookup(kind).is_none());
    }
    // Test-only construction exercises the proposed hook type, not registration.
    let proposed = Registration { kind: KIND, validate };
    assert_eq!(proposed.kind, KIND);
    let _: Validator = proposed.validate;
}


#[test]
fn valid_or_relayed_signature_without_post_sealing_proof_never_grants() {
    let (snapshot, claim, record, signature, _) = fixture();
    assert_eq!(refusal(decide(Some(&snapshot), &claim, &record, Some(&signature), Mode::Local)), Refusal::MissingPostSealingEvidence);
    // Even authentic remote history has no execution conversion. In particular,
    // carrying this signature in another writer's envelope cannot mint a grant.
    assert!(matches!(decide(Some(&snapshot), &claim, &record, Some(&signature), Mode::Replica), Ok(Disposition::HistoryOnly)));
}

