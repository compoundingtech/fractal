#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::arrangements_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::arrangements_tests_shared_seats::{A, B, SUBJECT, append, create, input};
use super::arrangements::*;
use super::*;
use st3_store::store::arrangements_tests_shared_attention_tests::{layout};


#[test]
fn owner_admission_real_agent_and_atomic_identity_fences() {
    let store = Store::open_memory("node").unwrap();
    for actor in [None, Some("person/other"), Some("daemon/runtime")] {
        let mut claim = input(json!([{"op":"create","name":"Work"}]));
        claim.actor = actor.map(str::to_owned);
        assert_eq!(store.append_client_claim(&claim).unwrap_err().code,"arrangement-owner-forbidden");
    }
    let mut wrong_owner = input(json!([{"op":"create","name":"Work"}]));
    wrong_owner.fields.insert("owner".into(),json!("person/other"));
    assert_eq!(store.append_claim(&wrong_owner).unwrap_err().code,"arrangement-owner-forbidden");
    create(&store);
    let before = layout(&store);
    let mut agent = input(json!([{"op":"rename","name":"From agent"}]));
    agent.actor = Some("agent/fleet/fixture-arrangements/seat".into());
    agent.idempotency_key = Some("session/edit-one".into());
    agent.fields.insert("action_id".into(),json!("action-one"));
    agent.fields.insert("action_digest".into(),json!("0".repeat(64)));
    let authority = store.append_claim(&ClaimInput { subject:"resource/authority".into(),kind:"resource.observed".into(),actor:None,
        fields:serde_json::from_value(json!({"kind":"custom.test.authority"})).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:None }).unwrap();
    let mut fences = BTreeMap::from([(SUBJECT.into(),"stale-layout".into()),(authority.subject.clone(),"stale-authority".into())]);
    assert_eq!(store.edit_arrangement(&agent,&fences).unwrap_err().code,"stale-subject");
    assert_eq!(layout(&store),before);
    fences.insert(authority.subject.clone(),authority.id.clone());
    agent.expected_subject = Some(Some("stale-layout".into()));
    let accepted = store.edit_arrangement(&agent,&fences).unwrap();
    assert_eq!(accepted.actor.as_deref(),Some("agent/fleet/fixture-arrangements/seat"));
    assert_eq!(layout(&store)["owner"],"person/ada");
    assert_eq!(layout(&store)["body"]["name"],json!({"value":"From agent","revision":accepted.id}));
    fences.insert(authority.subject.clone(),"now-stale-again".into());
    assert_eq!(store.edit_arrangement(&agent,&fences).unwrap().id,accepted.id);
    let connection = store.readers.get();
    let mut forged = accepted.clone();
    forged.actor = Some("person/other".into());
    assert!(classify_replicated_claim_with_registry(&forged,st3_schema::registry()).is_err());
    let status = subject_status_at(&connection,SUBJECT,None,None,crate::store::ClaimsReduction::Full).unwrap().unwrap().0;
    let mut changed_action = agent.clone();
    changed_action.fields.insert("action_id".into(),json!("action-two"));
    changed_action.fields.insert("action_digest".into(),json!("1".repeat(64)));
    assert_eq!(store.edit_arrangement(&changed_action,&fences).unwrap_err().code,"idempotency-mismatch");
    assert_eq!(status.owner_run,None);
    assert_eq!(status.actual.unwrap()["owner"],"person/ada");
}

#[test]
fn reads_and_write_validation_do_not_need_claim_history() {
    let store = Store::open_memory("node").unwrap();
    create(&store);
    let before = layout(&store);
    assert!(store.arrangement(SUBJECT,0).is_err());
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    // If either path consults edit history this fails, rather than merely asserting query text.
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h5b06528276f6 owner=st3_attention::store::arrangements_tests TODO=prepare-body-and-predecessor-values */ "ALTER TABLE claims RENAME TO unavailable_history").unwrap();
    assert_eq!(arrangement_at(&transaction,SUBJECT,u64::MAX).unwrap().unwrap(),before);
    assert_eq!(arrangements_at(&transaction,"person/ada",u64::MAX).unwrap(),vec![before.clone()]);
    prepare(&transaction,&input(json!([{"op":"folder.rename","id":A,"name":"New"}]))).unwrap();
    assert_eq!(subject_status_at(&transaction,SUBJECT,None,None,crate::store::ClaimsReduction::Full).unwrap().unwrap().0.actual,Some(before));
    transaction.rollback().unwrap();
}
