//! Whether an incarnation was ever ready is read from that incarnation's observations alone.
use super::*;
use smallclaims::sqlite::work::SqliteWorkScope;

fn observe(store: &Store, subject: &str, turn: usize, incarnation: &str, state: &str, reason: Option<&str>) {
    let mut fields = BTreeMap::from([
        ("state".into(), json!(state)),
        ("incarnation_id".into(), json!(incarnation)),
        ("driver".into(), json!("claude")),
        ("transition_sequence".into(), json!(turn)),
    ]);
    if let Some(reason) = reason {
        fields.insert("reason".into(), json!(reason));
    }
    store
        .append_claim(&ClaimInput {
            subject: subject.into(),
            kind: "harness.observed".into(),
            actor: Some(subject.into()),
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

#[test]
fn harness_was_ready_does_not_read_earlier_incarnations() {
    let steps = |earlier: usize| {
        let store = Store::open_memory("fixture").unwrap();
        let subject = "agent/example/ready";
        for turn in 0..earlier {
            observe(&store, subject, turn, &format!("old-{}", turn / 50), "working", None);
        }
        observe(&store, subject, earlier, "current", "starting", None);
        observe(&store, subject, earlier + 1, "current", "ready", Some("providerAuth"));
        observe(&store, subject, earlier + 2, "login", "ready", Some("providerAuth"));
        observe(&store, subject, earlier + 3, "current", "idle", None);
        let scope = SqliteWorkScope::start();
        assert!(store.harness_was_ready(subject, "current").unwrap());
        assert!(!store.harness_was_ready(subject, "login").unwrap());
        assert!(!store.harness_was_ready(subject, "missing").unwrap());
        scope.finish().vm_steps
    };
    let (small, large) = (steps(50), steps(1000));
    assert!(large <= small + 100, "earlier incarnations were read: small={small} large={large}");
}
