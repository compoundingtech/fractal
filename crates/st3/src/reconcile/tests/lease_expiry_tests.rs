use super::*;
use crate::model::WorkRequest;

struct Clock;
impl Clock {
    fn at(store: &Store, at: u128) -> Self {
        smallclaims::store::set_thread_clock(Some(at));
        store.set_write_clock_at(at).unwrap();
        Self
    }
}
impl Drop for Clock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

#[test]
fn lease_expiry_notifies_the_previous_claimer_once_when_reconciliation_releases_it() {
    // The expiry must also be in the past for the transaction clock.
    const START: u128 = 1_700_000_000_000;
    let store = Arc::new(Store::open_memory("node").unwrap());
    let _clock = Clock::at(&store, START);
    apply_source(
        &store,
        r#"version 2
agent "worker" { workspace "/tmp"; command "true"; restart "never" }
mission "lease" state="ready" {
  goal "Tell the previous worker why its claim ended."
  step "work" { assigned-to "agent/node.worker" }
}
"#,
        "lease-notice-publish",
    );
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "lease".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: BTreeMap::new(),
            idempotency_key: "lease-notice-run".into(),
        })
        .unwrap();
    let subject = &run.steps[0].subject;
    store.set_step_state(subject, "ready", None).unwrap();
    let request = |key: &str| WorkRequest {
        actor: Some("agent/node.worker".into()),
        incarnation: Some("current".into()),
        summary: None,
        reason: None,
        evidence: Vec::new(),
        wait_ms: None,
        idempotency_key: key.into(),
    };
    store
        .work_action(subject, "claim", &request("lease-notice-claim"))
        .unwrap();
    let _renew_clock = Clock::at(&store, START + 60_000);
    store
        .work_action(subject, "renew", &request("lease-notice-renew"))
        .unwrap();
    let accepted: String = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT accepted_at_unix_ms FROM local_work_lease_renewals WHERE subject=?1",
            [subject],
            |row| row.get(0),
        )
        .unwrap();
    let renewal_time = chrono::DateTime::from_timestamp_millis(accepted.parse().unwrap())
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
    let _expiry_clock = Clock::at(&store, START + 660_001);
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    reconciler.evaluate_mission_runs().unwrap();
    reconciler.evaluate_mission_runs().unwrap();
    let released = store
        .latest_claim(subject, Some("step-run.state"))
        .unwrap()
        .unwrap();
    assert_eq!(released.body["fields"]["status"], "ready");
    assert!(
        released.body["fields"]["reason"]
            .as_str()
            .unwrap()
            .contains("last accepted renewal")
    );
    let view = store.step_run(subject).unwrap().unwrap();
    assert_eq!(view.status, "ready");
    assert_eq!(view.attempt, 1);
    let messages = store.messages(Some("agent/node.worker"), true).unwrap();
    let notices: Vec<_> = messages
        .iter()
        .filter(|message| message.title.as_deref() == Some("Work claim expired"))
        .collect();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].content.contains("2023-11-14T22:24:20Z"));
    assert!(notices[0].content.contains(&renewal_time));
    assert!(notices[0].content.contains("attempt 1 unchanged"));
    assert!(
        notices[0]
            .content
            .contains(&format!("st work show {subject}"))
    );
}
