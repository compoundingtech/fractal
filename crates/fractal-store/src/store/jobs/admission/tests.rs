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

#[test]
fn signature_nonce_and_full_binding_corruption_refuse() {
    let (snapshot, claim, record, signature, _) = fixture();
    for change in 0..5 {
        let mut corrupted = signature.clone();
        match change {
            0 => corrupted.signature.push('!'),
            1 => corrupted.nonce.push('!'),
            2 => corrupted.signer = "host/forged".into(),
            3 => corrupted.format = Some("fields-v1".into()),
            _ => corrupted.on_behalf = Some("host/other".into()),
        }
        assert_eq!(refusal(decide(Some(&snapshot), &claim, &record, Some(&corrupted), Mode::Local)), Refusal::InvalidSignature);
    }
    for field in ["job_id", "execution_host", "authority_declaration"] {
        let mut corrupted = claim.clone();
        corrupted.body["fields"]["record"]["binding"][field] = json!("changed");
        let record = serde_json::from_value(corrupted.body["fields"]["record"].clone()).unwrap();
        assert!(decide(Some(&snapshot), &corrupted, &record, Some(&signature), Mode::Local).is_err());
    }
}

#[test]
fn membership_root_staleness_binding_and_origin_refuse() {
    let (mut snapshot, mut claim, record, signature, _) = fixture();
    snapshot.member.as_mut().unwrap().key = "different-root".into();
    assert_eq!(refusal(decide(Some(&snapshot), &claim, &record, Some(&signature), Mode::Local)), Refusal::UnknownHost);
    snapshot.member.as_mut().unwrap().key = signature.key.clone();
    snapshot.member.as_mut().unwrap().current = false;
    assert_eq!(refusal(decide(Some(&snapshot), &claim, &record, Some(&signature), Mode::Local)), Refusal::StaleAuthority);
    snapshot.member.as_mut().unwrap().current = true;
    let mut other = binding(&record).clone();
    other.execution_host = "different-host".into();
    snapshot.existing_binding = Some(other);
    assert_eq!(refusal(decide(Some(&snapshot), &claim, &record, Some(&signature), Mode::Local)), Refusal::ConflictingBinding);
    snapshot.existing_binding = None;
    claim.origin = "relaying-writer".into();
    assert_eq!(refusal(decide(Some(&snapshot), &claim, &record, Some(&signature), Mode::Local)), Refusal::OriginMismatch);
}
#[test]
fn snapshot_lookup_failure_is_not_absence_and_identity_is_not_a_request_field() {
    let store = crate::store::Store::open_memory("real-fixture-node").unwrap();
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    let (mut snapshot, mut claim, record, _, key) = fixture();
    let tx = connection.transaction().unwrap();
    assert!(matches!(Snapshot::read(&tx, &store, binding(&record)), Err(Refusal::LookupUnavailable)));
    tx.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    assert!(matches!(Snapshot::read(&tx, &store, binding(&record)), Err(Refusal::LookupUnavailable)));
    tx.execute_batch("CREATE TABLE job_admission_bindings(job_id TEXT PRIMARY KEY,binding TEXT NOT NULL);").unwrap();
    let actual = Snapshot::read(&tx, &store, binding(&record)).unwrap();
    assert!(actual.existing_binding.is_none());
    assert_eq!(actual.local_origin, "real-fixture-node");
    snapshot.local_origin = actual.local_origin;
    claim.body["fields"]["local_origin"] = json!("fixture-host");
    let signature = sign(&key, &claim);
    assert_eq!(refusal(decide(Some(&snapshot), &claim, &record, Some(&signature), Mode::Local)), Refusal::OriginMismatch);
    tx.execute("INSERT INTO job_admission_bindings VALUES(?1,'malformed')", [&claim.subject]).unwrap();
    assert!(matches!(Snapshot::read(&tx, &store, binding(&record)), Err(Refusal::LookupUnavailable)));
}

#[test]
fn replay_uses_claim_facts_without_membership_host_or_clock_lookups() {
    let store = crate::store::Store::open_memory("unrelated-fixture-node").unwrap();
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    let (_, claim, _, signature, _) = fixture();
    let tx = connection.transaction().unwrap(); // No tables: any snapshot lookup fails.
    for _ in 0..2 {
        assert!(matches!(validate(&tx, &store, &claim, Some(&signature), Mode::Replay), Ok(Disposition::HistoryOnly)));
    }
    assert_eq!(tx.total_changes(), 0);
}

#[test]
fn concurrent_conflicting_attempts_produce_zero_grants_until_proof_exists() {
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let attempts = (0..2).map(|index| {
            let barrier = &barrier;
            scope.spawn(move || {
                let (mut snapshot, mut claim, mut record, _, key) = fixture();
                let JobRecord::Transition { binding, .. } = &mut record else { unreachable!() };
                binding.authority_declaration = format!("claim/member-{index}");
                snapshot.member.as_mut().unwrap().declaration = binding.authority_declaration.clone();
                claim.body["fields"]["record"] = serde_json::to_value(&record).unwrap();
                let signature = sign(&key, &claim);
                barrier.wait();
                assert_eq!(refusal(decide(Some(&snapshot), &claim, &record, Some(&signature), Mode::Local)), Refusal::MissingPostSealingEvidence);
            })
        }).collect::<Vec<_>>();
        for attempt in attempts { attempt.join().unwrap(); }
    });
}

#[test]
fn step_lease_alone_never_establishes_host_authority() {
    let (snapshot, mut claim, record, _, _) = fixture();
    claim.body["fields"]["step_lease"] = json!({"claimant":"host/fixture-host"});
    assert_eq!(refusal(decide(Some(&snapshot), &claim, &record, None, Mode::Local)), Refusal::MissingSignature);
}
