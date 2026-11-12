use super::*;
use crate::store::glasses_tests_shared_delivery::sync;

const ADA: &str = "person/ada";
const SUBJECT: &str = "read-position/person/ada";
const SEAT: &str = "agent/fleet/fixture-seat";
const SESSION: &str = "session/019a0000-0000-7000-8000-000000000001";

fn advance(conversation: &str, at: u64) -> ClaimInput {
    ClaimInput {
        subject: SUBJECT.into(),
        kind: KIND.into(),
        actor: Some(ADA.into()),
        fields: serde_json::from_value(json!({"owner":ADA,"conversation":conversation,"at_ms":at})).unwrap(),
        evidence: vec![],
        expected_subject: None,
        idempotency_key: None,
    }
}

fn positions(store: &Store) -> Vec<Value> {
    store.read_snapshot(|index| read_positions_at(&store.readers.get(), ADA, index)).unwrap()
}

fn at_of(store: &Store, conversation: &str) -> Option<u64> {
    position_at_ms(&store.readers.get(), ADA, conversation).unwrap()
}

#[test]
fn appends_keep_the_largest_position_in_either_order() {
    let forward = Store::open_memory("read-forward").unwrap();
    let larger = forward.append_claim(&advance(SEAT, 2_000)).unwrap();
    forward.append_claim(&advance(SEAT, 1_000)).unwrap();
    let reverse = Store::open_memory("read-reverse").unwrap();
    reverse.append_claim(&advance(SEAT, 1_000)).unwrap();
    reverse.append_claim(&advance(SEAT, 2_000)).unwrap();
    for store in [&forward, &reverse] {
        assert_eq!(at_of(store, SEAT), Some(2_000));
        let rows = positions(store);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], format!("{SUBJECT}/{SEAT}"));
        assert_eq!(rows[0]["kind"], "read-position");
        assert_eq!(rows[0]["owner"], ADA);
        assert_eq!(rows[0]["conversation"], SEAT);
        assert_eq!(rows[0]["at"], "1970-01-01T00:00:02.000Z");
    }
    // The stale advance neither replaced the winner nor its revision.
    assert_eq!(positions(&forward)[0]["revision"], larger.id);
}

#[test]
fn replicated_and_appended_advances_converge_in_every_delivery_order() {
    let left = Store::open_memory("read-left").unwrap();
    let right = Store::open_memory("read-right").unwrap();
    left.append_claim(&advance(SEAT, 3_000)).unwrap();
    left.append_claim(&advance(SESSION, 500)).unwrap();
    right.append_claim(&advance(SEAT, 4_000)).unwrap();
    right.append_claim(&advance(SESSION, 500)).unwrap();
    // Equal times: the larger canonical claim key wins on every node.
    let forward = Store::open_memory("read-replica-forward").unwrap();
    let reverse = Store::open_memory("read-replica-reverse").unwrap();
    sync(&left, &forward);
    sync(&right, &forward);
    sync(&right, &reverse);
    sync(&left, &reverse);
    // Each writer also receives the other's advances on top of its own appended ones.
    sync(&right, &left);
    sync(&left, &right);
    let expected = positions(&forward);
    assert_eq!(expected.len(), 2);
    assert_eq!(at_of(&forward, SEAT), Some(4_000));
    assert_eq!(at_of(&forward, SESSION), Some(500));
    for store in [&reverse, &left, &right] {
        assert_eq!(positions(store), expected);
    }
    for store in [&forward, &reverse, &left, &right] {
        store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
        assert_eq!(positions(store), expected, "rebuild keeps the merged maximum");
        store.rebuild_claim_projections().unwrap();
        assert_eq!(positions(store), expected, "the registered rebuild hook keeps it too");
    }
}

#[test]
fn only_winning_advances_move_the_change_index() {
    let store = Store::open_memory("read-changes").unwrap();
    let start = store.index().unwrap();
    store.append_claim(&advance(SEAT, 2_000)).unwrap();
    let advanced = store.index().unwrap();
    assert!(changed(&store.readers.get(), start, advanced).unwrap());
    store.append_claim(&advance(SEAT, 1_500)).unwrap();
    let stale = store.index().unwrap();
    assert!(stale > advanced);
    assert!(!changed(&store.readers.get(), advanced, stale).unwrap(), "a stale advance changes nothing");
    store.append_claim(&advance(SESSION, 1)).unwrap();
    let other = store.index().unwrap();
    assert!(changed(&store.readers.get(), stale, other).unwrap());
    assert!(!changed(&store.readers.get(), other, other).unwrap());
}

#[test]
fn admission_refuses_other_owners_and_malformed_positions() {
    let store = Store::open_memory("read-admission").unwrap();
    let mut other_actor = advance(SEAT, 1);
    other_actor.actor = Some("person/robin".into());
    assert_eq!(store.append_claim(&other_actor).unwrap_err().code, "read-position-owner-forbidden");
    let mut other_owner = advance(SEAT, 1);
    other_owner.fields.insert("owner".into(), json!("person/robin"));
    assert_eq!(store.append_claim(&other_owner).unwrap_err().code, "read-position-owner-forbidden");
    let mut bad_conversation = advance("no-family", 1);
    bad_conversation.fields.insert("conversation".into(), json!("has space/x y"));
    assert_eq!(store.append_claim(&bad_conversation).unwrap_err().code, "invalid-read-position");
    let mut wrong_kind = advance(SEAT, 1);
    wrong_kind.kind = "glass.deleted".into();
    assert!(store.append_claim(&wrong_kind).is_err());
    // A trusted fleet agent may advance for the person, as for arrangements.
    let mut agent = advance(SEAT, 7);
    agent.actor = Some("agent/fleet/fixture-helper".into());
    store.append_claim(&agent).unwrap();
    assert_eq!(at_of(&store, SEAT), Some(7));
    assert!(positions(&store).iter().all(|row| row["owner"] == ADA));
}

#[test]
fn replicated_admission_refuses_another_person_advancing_the_owner() {
    let store = Store::open_memory("read-replica-admission").unwrap();
    let mut claim = store.append_claim(&advance(SEAT, 1)).unwrap();
    claim.actor = Some("person/robin".into());
    let error = classify_replicated_claim_with_registry(&claim, fractal_schema::registry())
        .err().expect("another person cannot advance the owner's position");
    assert_eq!(error.code, "read-position-owner-forbidden");
}
