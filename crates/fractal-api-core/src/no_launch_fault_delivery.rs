//! Real objective episodes reach a no-launch owner only on its declared member.
use crate::store::Store;
use crate::graph::parse_intent;
use crate::gate_check::now_ms;
use crate::reconcile::{Reconciler, tests::FakeRuntime};
use std::sync::Arc;
use tokio::sync::Notify;
use crate::store::fleet_admission_tests_shared_seats::{admit, key, node, sync};
use serde_json::json;

const OWNER: &str = "agent/example/slo-owner";
const OBJECTIVE: &str = r#"
objective "example/service" {
  about "Service budget owner"
  indicator "native" { objective "person-read"; }
  on-breach "fault" owner="agent/example/slo-owner" condition="service-burn"
}
"#;

fn declare(store: &Store, owner: &str) {
    let source = format!("version 2\n{owner}\n{OBJECTIVE}");
    let intent = parse_intent(&source, store.origin()).unwrap();
    let planned = store.mission(&intent, crate::model::IntentInput {
        kdl: source, source_name: None,
    }).unwrap();
    store.apply(&intent, &planned.subject_tokens, "no-launch-owner").unwrap();
    let owner = store.desired_subjects().unwrap().into_iter()
        .find(|subject| subject.subject == OWNER).unwrap();
    assert!(owner.member.is_none(), "delivery cannot depend on a runtime member");
}

fn reading(store: &Store, status: &str) -> crate::objective_faults::ObjectiveFaults {
    let rows = crate::declared_objectives::rows(store, u64::try_from(now_ms()).unwrap(), &[json!({
        "id": "person-read", "kind": "objective", "status": status,
        "target": "1% bad events", "current": "budget evidence", "explain": "Closed evidence window."
    })]).unwrap();
    crate::declared_objectives::check(store, &rows).unwrap()
}

fn reconciler(store: Arc<Store>) -> Reconciler<FakeRuntime> {
    let host = store.origin().to_owned();
    Reconciler::new(store, Arc::new(FakeRuntime::default()), host, Arc::new(Notify::new()))
}

fn deliver(reconciler: &Reconciler<FakeRuntime>) {
    reconciler.reconcile_once().unwrap();
}

#[test]
fn signed_members_deliver_one_message_per_burn_episode_only_at_the_no_launch_owner() {
    for owner in [
        "agent \"example/slo-owner\" { host \"birch\"; name \"Budget owner\"; description \"Own service burn episodes\"; }",
        "host \"birch\" { agent \"example/slo-owner\" {} }",
    ] {
        let anchor = key();
        let source = Arc::new(node("alder", Some(&anchor), Some(&anchor)));
        admit(&source, "alder", &anchor, "anchor", None);
        let identity = key();
        admit(&source, "birch", &identity, "invite", None);
        let destination = Arc::new(node("birch", Some(&identity), Some(&anchor)));
        declare(&source, owner);
        assert_eq!(source.agent_host(OWNER).unwrap().as_deref(), Some("birch"));
        assert_eq!(reading(&source, "burning").opened, 1);
        assert_eq!(reading(&source, "exhausted").opened, 0);
        sync(&source, &destination);
        assert_eq!(destination.agent_host(OWNER).unwrap().as_deref(), Some("birch"));
        let remote = reconciler(source.clone());
        let local = reconciler(destination.clone());
        for _ in 0..3 { deliver(&remote); }
        assert!(source.messages(Some(OWNER), true).unwrap().is_empty(), "foreign host cannot originate mail");
        for _ in 0..3 { deliver(&local); }
        let first = destination.messages(Some(OWNER), true).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(destination.claims_for(&first[0].subject, Some("message.sent")).unwrap().len(), 1);
        sync(&destination, &source);
        deliver(&remote);
        assert_eq!(source.messages(Some(OWNER), true).unwrap().len(), 1, "replication cannot cause another send");
        assert_eq!(reading(&source, "no-data").recovered, 0);
        assert_eq!(reading(&source, "met").recovered, 1);
        assert_eq!(reading(&source, "burning").opened, 1);
        sync(&source, &destination);
        deliver(&remote);
        assert_eq!(source.messages(Some(OWNER), true).unwrap().len(), 1);
        for _ in 0..3 { deliver(&local); }
        assert_eq!(destination.messages(Some(OWNER), true).unwrap().len(), 2, "a new episode gets its own durable message");
        sync(&destination, &source);
        assert_eq!(source.messages(Some(OWNER), true).unwrap().len(), 2);
    }
}

#[test]
fn no_launch_owner_message_survives_reopen_without_another_send() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("graph.db");
    {
        let store = Arc::new(Store::open(&path, "birch").unwrap());
        declare(&store, "agent \"example/slo-owner\" { host \"local\"; }");
        assert_eq!(store.agent_host(OWNER).unwrap().as_deref(), Some("birch"));
        assert_eq!(reading(&store, "burning").opened, 1);
        let local = reconciler(store.clone());
        deliver(&local);
        assert_eq!(store.messages(Some(OWNER), true).unwrap().len(), 1);
    }
    let store = Arc::new(Store::open(&path, "birch").unwrap());
    let first = store.messages(Some(OWNER), true).unwrap().remove(0);
    let local = reconciler(store.clone());
    for _ in 0..3 { deliver(&local); }
    let mail = store.messages(Some(OWNER), true).unwrap();
    assert_eq!(mail.len(), 1);
    assert_eq!(mail[0].subject, first.subject);
    assert_eq!(store.claims_for(&first.subject, Some("message.sent")).unwrap().len(), 1);
}
