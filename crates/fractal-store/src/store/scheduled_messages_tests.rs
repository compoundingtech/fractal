use super::*;
use crate::store::fleet_admission_tests::{fleet, key, sync};

const SENDER: &str = "agent/example/sender";
const TARGET: &str = "agent/example/target";
const AT: u128 = 1_800_000_000_000;

fn setup() -> (Store, Store) {
    let (alder, birch) = (key(), key());
    let (_, _, mut stores) = fleet(&[("alder", &alder), ("birch", &birch)]);
    let other = stores.pop().unwrap();
    let owner = stores.pop().unwrap();
    let intent = crate::parse_intent(&format!(
        "version 2\nnotice-owner {{ member \"alder\"; member-key \"{}\"; always-on }}\nagent \"example/target\" {{ command \"true\"; workspace \"/tmp\"; }}", alder.public()), "alder").unwrap();
    let expected = intent.subjects.keys().map(|subject| (subject.clone(), vec![])).collect();
    owner.apply_as(&intent, &expected, "scheduled-setup", Some("person/example")).unwrap();
    sync(&owner, &other);
    (owner, other)
}

fn input(key: &str) -> ClaimInput {
    ClaimInput {
        subject: format!("message/{key}"), kind: "message.sent".into(), actor: Some(SENDER.into()),
        fields: BTreeMap::from([("from".into(),json!(SENDER)),("to".into(),json!(TARGET)),
            ("content".into(),json!("An invented reminder.")),("status".into(),json!("sent"))]),
        evidence: vec![], expected_subject: None, idempotency_key: Some(key.into()),
    }
}

fn sent(store: &Store) -> usize {
    store.readers.get().query_row("SELECT COUNT(*) FROM claims WHERE kind='message.sent'", [], |row| row.get(0)).unwrap()
}

#[test]
fn scheduled_message_never_early_and_delivered_once_with_receipt() {
    let (owner, _) = setup();
    let scheduled = owner.schedule_message(&input("never-early"), AT, "never-early").unwrap();
    assert_eq!(scheduled.state,"pending");
    assert_eq!(sent(&owner),0);
    let pass = owner.reconcile_scheduled_messages_at(AT-1).unwrap();
    assert!(!pass.changed);
    assert_eq!(pass.next_deadline,Some(AT));
    assert_eq!(sent(&owner),0);
    assert!(owner.reconcile_scheduled_messages_at(AT).unwrap().changed);
    assert_eq!(sent(&owner),1);
    assert!(!owner.reconcile_scheduled_messages_at(AT+1).unwrap().changed);
    let delivered = owner.scheduled_message(&scheduled.id).unwrap().unwrap();
    assert_eq!(delivered.state,"delivered");
    assert!(owner.message(delivered.message_id.as_deref().unwrap()).unwrap().is_some());
    assert_eq!(owner.readers.get().query_row("SELECT COUNT(*) FROM local_scheduled_message_pending", [], |row| row.get::<_,usize>(0)).unwrap(),0);
}

#[test]
fn scheduled_message_key_is_immutable_before_and_after_delivery() {
    let (owner, _) = setup();
    let original = input("immutable-key");
    let first = owner.schedule_message(&original, AT, "immutable-key").unwrap();
    assert_eq!(owner.schedule_message(&original, AT, "immutable-key").unwrap().id, first.id);
    let mut changed = original.clone();
    changed.fields.insert("content".into(),json!("A different note."));
    assert_eq!(owner.schedule_message(&changed, AT, "immutable-key").unwrap_err().code,"idempotency-mismatch");
    assert_eq!(owner.schedule_message(&original, AT+1, "immutable-key").unwrap_err().code,"idempotency-mismatch");
    assert_eq!(owner.append_claim(&original).unwrap_err().code,"idempotency-mismatch");
    owner.reconcile_scheduled_messages_at(AT).unwrap();
    assert_eq!(owner.schedule_message(&original, AT, "immutable-key").unwrap().state,"delivered");
    assert_eq!(owner.schedule_message(&changed, AT, "immutable-key").unwrap_err().code,"idempotency-mismatch");
    assert_eq!(sent(&owner),1);
}

#[test]
fn scheduled_message_cancel_is_authorized_and_serialized_with_delivery() {
    let (owner, _) = setup();
    for (key, actor) in [("sender-cancel",SENDER),("target-cancel",TARGET)] {
        let request = owner.schedule_message(&input(key), AT, key).unwrap();
        assert_eq!(owner.cancel_scheduled_message(&request.id,"agent/example/unrelated","not mine").unwrap_err().code,"scheduled-message-forbidden");
        assert_eq!(owner.cancel_scheduled_message(&request.id,actor,"no longer needed").unwrap().state,"cancelled");
        assert_eq!(owner.schedule_message(&input(key), AT, key).unwrap().state,"cancelled");
    }
    owner.reconcile_scheduled_messages_at(AT+1).unwrap();
    assert_eq!(sent(&owner),0);
    let delivered = owner.schedule_message(&input("late-cancel"), AT, "late-cancel").unwrap();
    owner.reconcile_scheduled_messages_at(AT).unwrap();
    assert_eq!(owner.cancel_scheduled_message(&delivered.id,SENDER,"too late").unwrap_err().code,"already-delivered");
    assert_eq!(sent(&owner),1);
}

#[test]
fn scheduled_message_replicas_adopt_one_owner_and_deterministic_mail_identity() {
    let (owner, other) = setup();
    let a = owner.schedule_message(&input("replica-key"), AT, "replica-key").unwrap();
    let b = other.schedule_message(&input("replica-key"), AT, "replica-key").unwrap();
    assert_eq!(a.id,b.id);
    sync(&other,&owner);
    sync(&owner,&other);
    assert!(!other.reconcile_scheduled_messages_at(AT).unwrap().changed);
    assert!(other.scheduled_message(&a.id).unwrap().unwrap().blocker.is_some());
    assert_eq!(sent(&other),0);
    assert_eq!(other.cancel_scheduled_message(&a.id,SENDER,"remote cancel must serialize at owner").unwrap_err().code,
        "scheduled-message-owner-pending");
    assert!(owner.reconcile_scheduled_messages_at(AT).unwrap().changed);
    sync(&owner,&other);
    assert!(!other.reconcile_scheduled_messages_at(AT+1).unwrap().changed);
    assert_eq!(owner.scheduled_message(&a.id).unwrap().unwrap().message_id,
        other.scheduled_message(&a.id).unwrap().unwrap().message_id);
    assert_eq!(sent(&other),1);
    assert_eq!(sent(&owner),1);
}

#[test]
fn scheduled_message_owner_unavailable_stays_pending_with_visible_blocker() {
    let store = Store::open_memory("alder").unwrap();
    let intent = crate::parse_intent("version 2\nagent \"example/target\" { command \"true\"; workspace \"/tmp\"; }", "alder").unwrap();
    store.apply_as(&intent,&BTreeMap::from([(TARGET.into(),vec![])]),"target-only",Some("person/example")).unwrap();
    let request = store.schedule_message(&input("no-owner"), AT, "no-owner").unwrap();
    assert!(request.blocker.is_some());
    let pass = store.reconcile_scheduled_messages_at(AT).unwrap();
    assert!(!pass.changed);
    assert_eq!(pass.next_deadline,None,"blocked requests must not become a polling timer");
    assert_eq!(store.scheduled_message(&request.id).unwrap().unwrap().state,"pending");
    assert_eq!(sent(&store),0);
}

#[test]
fn scheduled_message_retirement_cancels_instead_of_inheriting_an_old_seat() {
    let (owner, _) = setup();
    let request = owner.schedule_message(&input("retired-target"), AT, "retired-target").unwrap();
    let declaration = owner.selected_desired_token(TARGET).unwrap().unwrap();
    owner.connection.batched(|tx| append_admitted_claim_tx(tx,"alder",TARGET,"agent.retired",Some("person/example"),
        &json!({"fields":{"reason":"Finished","retired_by":"person/example","owner_host":"alder","declaration":declaration}}),
        &[],None,false)).unwrap().unwrap();
    owner.reconcile_scheduled_messages_at(AT).unwrap();
    let cancelled = owner.scheduled_message(&request.id).unwrap().unwrap();
    assert_eq!(cancelled.state,"cancelled");
    assert_eq!(cancelled.cancel_reason.as_deref(),Some("recipient-retired"));
    assert_eq!(sent(&owner),0);
}

#[test]
fn scheduled_message_generic_claim_writer_cannot_forge_delivery() {
    let (owner, _) = setup();
    let mut forged = input("forged");
    forged.subject = "message-schedule/forged".into();
    forged.kind = scheduled_messages::REQUESTED.into();
    assert!(owner.append_claim(&forged).is_err());
}

#[test]
fn scheduled_message_can_wake_its_own_existing_seat() {
    let (owner, _) = setup();
    let mut reminder = input("self-wake");
    reminder.actor = Some(TARGET.into());
    reminder.fields.insert("from".into(),json!(TARGET));
    let request = owner.schedule_message(&reminder, AT, "self-wake").unwrap();
    assert_eq!(request.from,request.to);
    assert_eq!(sent(&owner),0);
    owner.reconcile_scheduled_messages_at(AT).unwrap();
    assert_eq!(sent(&owner),1);
    assert_eq!(owner.scheduled_message(&request.id).unwrap().unwrap().state,"delivered");
}
