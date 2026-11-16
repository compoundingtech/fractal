//! Durable seat-message deadlines use the native timer and recheck wall time after every wake.
use super::*;
use crate::store::fleet_admission_tests_shared_seats::{FLEET, admit, key};
use futures_util::FutureExt as _;

const START: u128 = 1_800_000_000_000;
const TARGET: &str = "agent/example/target";
const CONTENT: &str = "The scheduled seat message is due.";

struct Clock;

impl Clock {
    fn at(at: u128) -> Self {
        smallclaims::store::set_thread_clock(Some(at));
        Self
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

fn seed_recipient(store: &Store, identity: &crate::fleet::MemberKey) {
    admit(store, "alder", identity, "anchor", None);
    store.project_replication_backlog().unwrap();
    store.fleet_membership().unwrap();
    apply_source(store, &format!(
        "version 2\nnotice-owner {{ member \"alder\"; member-key \"{}\"; always-on }}\nagent \"example/target\" {{ workspace \"/tmp\"; command \"true\" }}",
        identity.public(),
    ), "scheduled-message-recipient");
}

fn schedule(store: &Store) {
    store.schedule_message(&ClaimInput {
        subject: "message/scheduled-deadline-test".into(),
        kind: "message.sent".into(),
        actor: Some("agent/example/sender".into()),
        fields: BTreeMap::from([
            ("from".into(), serde_json::json!("agent/example/sender")),
            ("to".into(), serde_json::json!(TARGET)),
            ("content".into(), serde_json::json!(CONTENT)),
            ("status".into(), serde_json::json!("sent")),
            ("tags".into(), serde_json::json!([])),
        ]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }, START + 50, "scheduled-deadline-test").unwrap();
}

fn seed(store: &Store, identity: &crate::fleet::MemberKey) {
    seed_recipient(store, identity);
    schedule(store);
}

fn reconciler(store: Arc<Store>) -> Reconciler<FakeRuntime> {
    Reconciler::new(store, Arc::new(FakeRuntime::default()), "alder".into(), Arc::new(Notify::new()))
}

fn delivered(store: &Store) -> usize {
    store.messages(Some(TARGET), true).unwrap().iter()
        .filter(|message| message.content == CONTENT).count()
}

#[tokio::test(start_paused = true)]
async fn scheduled_message_timer_never_delivers_before_wall_clock_deadline() {
    let _clock = Clock::at(START);
    let identity = key();
    let store = Arc::new(Store::open_memory("alder").unwrap());
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(identity.public()).unwrap();
    store.set_member_key(Some(identity.clone())).unwrap();
    store.set_write_clock_at(START).unwrap();
    seed(&store, &identity);
    let reconciler = reconciler(store.clone());
    let (pass, due) = smallclaims::touched::record_due(|| reconciler.reconcile_scheduled_messages());
    assert!(!pass.unwrap());
    assert_eq!(due, Some(START + 50));
    assert_eq!(reconciler.step_deadlines.lock().unwrap().get("seat-message-deadline"), Some(&(START + 50)));
    assert_eq!(delivered(&store), 0);
    let _ = reconciler.notify.notified().now_or_never();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(49)).await;
    tokio::task::yield_now().await;
    assert!(reconciler.notify.notified().now_or_never().is_none());
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert!(reconciler.notify.notified().now_or_never().is_some());

    // The monotonic timer fired, but wall time moved back: wake is not delivery authority.
    smallclaims::store::set_thread_clock(Some(START + 49));
    assert!(!reconciler.reconcile_scheduled_messages().unwrap());
    assert_eq!(delivered(&store), 0);
    assert_eq!(reconciler.step_deadlines.lock().unwrap().get("seat-message-deadline"), Some(&(START + 50)));
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert!(reconciler.notify.notified().now_or_never().is_some());
    smallclaims::store::set_thread_clock(Some(START + 50));
    assert!(reconciler.reconcile_scheduled_messages().unwrap());
    assert_eq!(delivered(&store), 1);
    assert!(!reconciler.reconcile_scheduled_messages().unwrap());
    assert_eq!(delivered(&store), 1);
}

#[test]
fn scheduled_message_overdue_at_restart_is_delivered_by_the_startup_pass() {
    let _clock = Clock::at(START);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("graph.db");
    let identity = key();
    {
        let store = Arc::new(Store::open(&path, "alder").unwrap());
        store.bind_fleet(FLEET).unwrap();
        store.pin_fleet_anchor(identity.public()).unwrap();
        store.set_member_key(Some(identity.clone())).unwrap();
        store.set_write_clock_at(START).unwrap();
        seed(&store, &identity);
        let reconciler = reconciler(store.clone());
        assert!(!reconciler.reconcile_scheduled_messages().unwrap());
        assert_eq!(delivered(&store), 0);
    }
    smallclaims::store::set_thread_clock(Some(START + 500));
    let store = Arc::new(Store::open(&path, "alder").unwrap());
    store.set_member_key(Some(identity)).unwrap();
    store.set_write_clock_at(START + 500).unwrap();
    let reconciler = reconciler(store.clone());
    reconciler.reconcile_once().unwrap();
    assert_eq!(delivered(&store), 1, "startup recovers due work without a pre-existing timer");
    reconciler.reconcile_once().unwrap();
    assert_eq!(delivered(&store), 1, "the recovered request is delivered exactly once");
}

#[test]
fn scheduled_message_to_a_suspended_seat_queues_native_mail_without_resuming_it() {
    let _clock = Clock::at(START);
    let identity = key();
    let store = Arc::new(Store::open_memory("alder").unwrap());
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(identity.public()).unwrap();
    store.set_member_key(Some(identity.clone())).unwrap();
    store.set_write_clock_at(START).unwrap();
    seed_recipient(&store, &identity);
    let declaration = store.selected_desired_token(TARGET).unwrap().unwrap();
    let request = store.append_claim(&ClaimInput {
        subject: TARGET.into(),
        kind: "runtime.action.requested".into(),
        actor: Some("person/operator".into()),
        fields: BTreeMap::from([("action".into(), serde_json::json!("suspend"))]),
        evidence: vec![declaration],
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    // As in the rollout fixtures, seed the captured phase rather than workspace archives.
    store.append_claim(&ClaimInput {
        subject: TARGET.into(),
        kind: "runtime.action.succeeded".into(),
        actor: Some("person/operator".into()),
        fields: BTreeMap::from([
            ("action".into(), serde_json::json!("suspend")),
            ("operation_status".into(), serde_json::json!("suspended")),
        ]),
        evidence: vec![request.id.clone()],
        expected_subject: None,
        idempotency_key: Some(crate::suspension::suspend_completed_key(&request.id)),
    }).unwrap();
    let suspension = crate::suspension::current(&store, TARGET).unwrap().unwrap();
    assert_eq!(suspension.phase, "suspended");
    assert!(suspension.holds_seat());
    schedule(&store);
    assert_eq!(delivered(&store), 0);

    smallclaims::store::set_thread_clock(Some(START + 50));
    store.set_write_clock_at(START + 50).unwrap();
    let reconciler = reconciler(store.clone());
    reconciler.reconcile_once().unwrap();
    let messages = store.messages(Some(TARGET), true).unwrap();
    let queued = messages.iter().find(|message| message.content == CONTENT).unwrap();
    assert_eq!(queued.status, "sent", "ordinary native mail stays queued while suspended");
    assert!(store.latest_claim(&queued.subject, Some("message.sent")).unwrap().is_some());
    assert!(reconciler.runtime.starts.lock().unwrap().is_empty(), "scheduled mail must not start the suspended runtime");
    assert_eq!(crate::suspension::current(&store, TARGET).unwrap().unwrap(), suspension);
    reconciler.reconcile_once().unwrap();
    assert_eq!(delivered(&store), 1);
    assert!(reconciler.runtime.starts.lock().unwrap().is_empty());
}
