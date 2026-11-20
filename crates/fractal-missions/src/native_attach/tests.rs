use super::*;
use crate::PersonWorkStore;
use fractal_kernel::model::PersonStepResponse;
use fractal_runtime::native_attach::{self, AttachWitness, ProofListener};
use fractal_store::store::native_attach::{Candidate, Preparation};
use serde_json::json;
use smallclaims::claim::{ClaimInput, ClaimRecord};
use std::{fs::{self, File}, path::PathBuf, time::Duration};

const PERSON: &str = "person/operator";
const SESSION: &str = "person/operator/session/paired";
const NATIVE: &str = "full-native-id-not-a-prefix";

struct Fixture {
    _directory: tempfile::TempDir,
    _file: File,
    witness: AttachWitness,
    candidate: Candidate,
    challenge: String,
}
impl Fixture {
    fn new() -> Self {
        // Workspaces must be durable, not below the platform's temporary roots.
        let directory = tempfile::Builder::new().prefix("attach-test-").tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let path = root.join("transcript.jsonl");
        fs::write(&path, format!("{{\"type\":\"session\",\"id\":\"{NATIVE}\"}}\n")).unwrap();
        let file = File::open(&path).unwrap();
        let evidence = native_attach::inspect(std::process::id(), &path, NATIVE).unwrap();
        let socket_directory = tempfile::tempdir().unwrap();
        let mut listener = ProofListener::bind(socket_directory.path(), &evidence, Duration::from_secs(5)).unwrap();
        let challenge = listener.challenge_digest();
        let input = listener.challenge().clone();
        let report_path = path.clone();
        let reporting = std::thread::spawn(move || native_attach::report(&input, &report_path, NATIVE).unwrap());
        let witness = listener.accept().unwrap();
        reporting.join().unwrap();
        assert!(listener.accept().is_err(), "one-shot proof cannot be replayed");
        Self { _directory: directory, _file: file, witness, candidate: Candidate {
            host:"host/test".into(), agent:"agent/external".into(), person:PERSON.into(), evidence, workspace:root,
        }, challenge }
    }
    fn prepare(&self, store: &Store, key: &str) -> Preparation {
        store.prepare_native_attach(self.candidate.clone(), key, &self.challenge,
            smallclaims::store::now_ms() + 120_000, &self.witness).unwrap()
    }
}
fn grant(store: &Store, expiry: u64) -> ClaimRecord {
    store.append_client_operation_claim(&ClaimInput {
        subject:format!("custom/client/pairing-{}",uuid::Uuid::new_v4()),
        kind:"custom.client.pairing-completed".into(), actor:Some(PERSON.into()),
        fields:serde_json::from_value(json!({"person_id":PERSON,"session_actor":SESSION,
            "credential_hash":"fixture-only","expires_at_unix_ms":expiry,"scopes":["control.work"]})).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None,
    }).unwrap()
}
fn approve(store: &Store, prepared: &Preparation, paired: &ClaimRecord) -> String {
    let view = store.ask_native_attach(&prepared.id).unwrap();
    assert_eq!(view.subject,store.ask_native_attach(&prepared.id).unwrap().subject);
    let ask = store.claims_for_subject_kind_at(&view.subject,"work.person-asked",None,true,1).unwrap().claims.remove(0);
    assert_eq!(ask.body["fields"]["request"],prepared.request());
    let input: PersonStepResponse = serde_json::from_value(json!({"subject":view.subject,"actor":PERSON,
        "summary":"Approve this exact attachment","episode":ask.id,"answer":{"id":"attach"},
        "idempotency_key":format!("approve-{}",prepared.id)})).unwrap();
    assert!(store.finish_person_step(&input,false).is_err(),"local answer is not paired consent");
    store.finish_person_step_kind(&input,false,None,false,Some((&paired.subject,SESSION))).unwrap();
    store.claims_for_subject_kind_at(&view.subject,"work.person-done",None,true,1).unwrap().claims.remove(0).id
}

#[test]
fn exact_ask_commit_and_double_commit_preserve_external_ownership() {
    let fixture = Fixture::new();
    let store = Store::open_memory("host/test").unwrap();
    let prepared = fixture.prepare(&store,"prepare");
    assert_eq!(prepared.id,fixture.prepare(&store,"prepare").id);
    let answer = approve(&store,&prepared,&grant(&store,u64::MAX));
    let committed = store.commit_native_attach(&prepared.id,&prepared.candidate_digest,&answer,"commit",&fixture.witness).unwrap();
    let retried = store.commit_native_attach(&prepared.id,&prepared.candidate_digest,&answer,"commit",&fixture.witness).unwrap();
    assert_eq!(committed.id,retried.id);
    assert_eq!(serde_json::to_value(&committed.ownership).unwrap(),"external");
    assert_eq!(serde_json::to_value(&committed.phase).unwrap(),"channel-pending");
    assert!(store.commit_native_attach(&prepared.id,&prepared.candidate_digest,&answer,"another",&fixture.witness).is_err());
    assert_eq!(store.native_attach_binding(&prepared.id).unwrap().unwrap().id,committed.id);
    assert!(store.native_attach_preparation("forged/id").is_err());
}

#[test]
fn forged_identity_prefix_and_changed_native_id_cannot_rebind() {
    let fixture = Fixture::new();
    let store = Store::open_memory("host/test").unwrap();
    for field in ["pid","start","boot","uid","native","transcript"] {
        let mut candidate = fixture.candidate.clone();
        match field {
            "pid" => candidate.evidence.process.pid += 1,
            "start" => candidate.evidence.process.start_token += 1,
            "boot" => candidate.evidence.process.boot_id.push('x'),
            "uid" => candidate.evidence.process.uid += 1,
            "native" => candidate.evidence.transcript.native_id = "full-native".into(),
            "transcript" => candidate.evidence.transcript.path = PathBuf::from("/forged"),
            _ => unreachable!(),
        }
        assert!(store.prepare_native_attach(candidate,field,&fixture.challenge,smallclaims::store::now_ms()+120_000,&fixture.witness).is_err(),"{field}");
    }
    let prepared = fixture.prepare(&store,"original");
    let answer = approve(&store,&prepared,&grant(&store,u64::MAX));
    let path = &fixture.candidate.evidence.transcript.path;
    fs::write(path,"{\"type\":\"session\",\"id\":\"another-native-session\"}\n").unwrap();
    assert!(store.commit_native_attach(&prepared.id,&prepared.candidate_digest,&answer,"commit",&fixture.witness).is_err(),"native session switches require fresh approval");
}

#[test]
fn consumed_answer_and_candidate_reservations_are_atomic_under_competing_prepares() {
    let fixture = Fixture::new();
    let store = std::sync::Arc::new(Store::open_memory("host/test").unwrap());
    let first = fixture.prepare(&store,"first");
    let second = fixture.prepare(&store,"second");
    let paired = grant(&store,u64::MAX);
    let first_answer = approve(&store,&first,&paired);
    let second_answer = approve(&store,&second,&paired);
    assert!(store.commit_native_attach(&second.id,&second.candidate_digest,&first_answer,"stolen-answer",&fixture.witness).is_err());
    std::thread::scope(|scope| {
        let left = scope.spawn(|| fixture.prepare(&store,"racing-prepare"));
        let right = scope.spawn(|| fixture.prepare(&store,"racing-prepare"));
        assert_eq!(left.join().unwrap().id,right.join().unwrap().id);
    });
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let attempt = |prepared: &Preparation, answer: &str| {
            barrier.wait();
            store.commit_native_attach(&prepared.id,&prepared.candidate_digest,answer,"commit",&fixture.witness).is_ok()
        };
        let one = scope.spawn(|| attempt(&first,&first_answer));
        let two = scope.spawn(|| attempt(&second,&second_answer));
        assert_ne!(one.join().unwrap(),two.join().unwrap(),"exactly one process/session/agent reservation wins");
    });
}

#[test]
fn expiry_and_generic_claim_forgery_never_authorize_attach() {
    let fixture = Fixture::new();
    let store = Store::open_memory("host/test").unwrap();
    let mut forged_host = fixture.candidate.clone();
    forged_host.host = "host/forged".into();
    assert!(store.prepare_native_attach(forged_host,"forged-host",&fixture.challenge,smallclaims::store::now_ms()+120_000,&fixture.witness).is_err());
    assert!(store.prepare_native_attach(fixture.candidate.clone(),"forged-proof",&"0".repeat(64),smallclaims::store::now_ms()+120_000,&fixture.witness).is_err());
    let prepared = fixture.prepare(&store,"prepare");
    let expired = grant(&store,1);
    let view = store.ask_native_attach(&prepared.id).unwrap();
    let ask = store.claims_for_subject_kind_at(&view.subject,"work.person-asked",None,true,1).unwrap().claims.remove(0);
    let input = serde_json::from_value(json!({"subject":view.subject,"actor":PERSON,"summary":"Attach",
        "episode":ask.id,"answer":{"id":"attach"},"idempotency_key":"expired"})).unwrap();
    assert!(store.finish_person_step_kind(&input,false,None,false,Some((&expired.subject,SESSION))).is_err());
    for kind in ["custom.native-attach.prepared","custom.native-attach.ask-bound","custom.native-attach.committed"] {
        assert!(store.append_claim(&ClaimInput {subject:prepared.subject(),kind:kind.into(),actor:Some(REQUESTER.into()),
            fields:serde_json::Map::new(),evidence:vec![],expected_subject:None,idempotency_key:None}).is_err());
    }
    assert!(store.prepare_native_attach(fixture.candidate.clone(),"expired",&fixture.challenge,smallclaims::store::now_ms(),&fixture.witness).is_err());
    let brief = store.prepare_native_attach(fixture.candidate.clone(),"brief",&fixture.challenge,smallclaims::store::now_ms()+10,&fixture.witness).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    assert!(store.ask_native_attach(&brief.id).is_err());
}

#[test]
fn paired_grant_must_still_be_live_at_commit() {
    let fixture = Fixture::new();
    let store = Store::open_memory("host/test").unwrap();
    let prepared = fixture.prepare(&store,"prepare");
    let paired = grant(&store,u64::try_from(smallclaims::store::now_ms()).unwrap()+2_000);
    let answer = approve(&store,&prepared,&paired);
    std::thread::sleep(Duration::from_secs(2));
    assert!(store.commit_native_attach(&prepared.id,&prepared.candidate_digest,&answer,"commit",&fixture.witness).is_err());
    assert!(store.native_attach_binding(&prepared.id).unwrap().is_none());
}

#[test]
fn committed_external_incarnation_survives_store_reopen() {
    let fixture = Fixture::new();
    let database = fixture.candidate.workspace.join("store.db");
    let store = Store::open(&database,"host/test").unwrap();
    let prepared = fixture.prepare(&store,"prepare");
    let answer = approve(&store,&prepared,&grant(&store,u64::MAX));
    let committed = store.commit_native_attach(&prepared.id,&prepared.candidate_digest,&answer,"commit",&fixture.witness).unwrap();
    drop(store);
    let reopened = Store::open(&database,"host/test").unwrap();
    assert_eq!(reopened.native_attach_binding(&prepared.id).unwrap().unwrap().id,committed.id);
    assert_eq!(reopened.commit_native_attach(&prepared.id,&prepared.candidate_digest,&answer,"commit",&fixture.witness).unwrap().id,committed.id);
}
