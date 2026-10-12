use super::*;

const CHILD: &str = "ST3_FAULT_PHASE_SPANS_CHILD";
const AT: u128 = 1_800_000_000_000;

struct FixedClock;
impl Drop for FixedClock {
    fn drop(&mut self) {
        smallclaims::store::set_thread_clock(None);
    }
}

#[test]
fn profiled_fault_delivery_keeps_snapshot_and_message_semantics() {
    // Profiling is process-global. Keep this control out of parallel tests and leave their
    // environment untouched; the child runs just this named test with its own profile files.
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("profiled_fault_delivery_keeps_snapshot_and_message_semantics")
            .arg("--nocapture")
            .env(CHILD, "1")
            .env("ST3_PROFILE_DIR", directory.path())
            .env("ST3_PROFILE_SLOW_MS", "0")
            .status()
            .unwrap();
        assert!(status.success(), "isolated fault profile control failed");
        return;
    }

    crate::profile::init_from_env();
    smallclaims::store::set_thread_clock(Some(AT));
    let _clock = FixedClock;
    let store = Arc::new(Store::open_memory("node").unwrap());
    store.set_write_clock_at(AT).unwrap();
    apply_source(
        &store,
        r#"version 2
agent "node.ops" { workspace "/tmp"; command "true"; handles-faults }
agent "node.other" { workspace "/tmp"; command "true" }
"#,
        "fault-phase-profile-agents",
    );
    let desired = store.desired_subjects().unwrap();
    assert!(desired.iter().any(|item| {
        item.subject == "agent/node.ops"
            && item.member.as_ref().is_some_and(|member| member.host == "node")
    }));
    let mut reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    assert!(store.fault_snapshot(AT).unwrap().is_empty());
    crate::profile::task("test/fault-empty", || {
        reconciler.deliver_faults(&desired).unwrap();
    });
    store
        .record_operational_failure(
            "disk",
            &AttentionRequest {
                reviewer: "person/operator".into(),
                title: "Disk space is low".into(),
                reason: "The disk is nearly full.".into(),
                severity: "error".into(),
                targets: vec!["daemon/node".into()],
                actor: "daemon/runtime".into(),
                idempotency_key: "fault-phase-profile-disk".into(),
            },
        )
        .unwrap();
    let before = serde_json::to_value(store.fault_snapshot(AT).unwrap()).unwrap();
    assert_eq!(before.as_array().unwrap().len(), 1);

    crate::profile::task("test/fault-phase", || {
        reconciler.deliver_faults(&desired).unwrap();
    });
    assert_eq!(serde_json::to_value(store.fault_snapshot(AT).unwrap()).unwrap(), before);
    let fault_messages = |agent: &str| {
        store
            .messages(Some(agent), false)
            .unwrap()
            .into_iter()
            .filter(|message| message.title.as_deref() == Some("Fault: Disk space is low"))
            .map(|message| (message.subject, message.content))
            .collect::<Vec<_>>()
    };
    let sent = fault_messages("agent/node.ops");
    assert_eq!(sent.len(), 1);
    assert!(fault_messages("agent/node.other").is_empty());

    // A second delivery uses the same episode and must not create a second message.
    reconciler.deliver_faults(&desired).unwrap();
    assert_eq!(fault_messages("agent/node.ops"), sent);

    // No local agent leaves before either child span. A non-owner local agent still gets a
    // snapshot and owner-filter span, but does not receive this fault.
    crate::profile::task("test/fault-no-agent", || {
        reconciler.deliver_faults(&[]).unwrap();
    });
    let other_only = desired
        .iter()
        .filter(|item| item.subject == "agent/node.other")
        .cloned()
        .collect::<Vec<_>>();
    crate::profile::task("test/fault-nonowner", || {
        reconciler.deliver_faults(&other_only).unwrap();
    });
    assert!(fault_messages("agent/node.other").is_empty());
    reconciler.fault_delivery_every_ms = 60_000;
    crate::profile::task("test/fault-throttled", || {
        reconciler.deliver_faults(&desired).unwrap();
    });
    assert_eq!(fault_messages("agent/node.ops"), sent);

    let lines = std::fs::read_to_string(
        std::path::Path::new(&std::env::var_os("ST3_PROFILE_DIR").unwrap()).join("slow.jsonl"),
    )
    .unwrap();
    let records = lines
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let spans = |label: &str| {
        records
            .iter()
            .find(|record| record["label"] == label)
            .unwrap()["spans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|span| (span["name"].as_str().unwrap().to_owned(), span["count"].as_u64().unwrap()))
            .collect::<BTreeMap<_, _>>()
    };
    let delivered = spans("test/fault-phase");
    assert_eq!(delivered.get("fault.snapshot"), Some(&1));
    assert_eq!(delivered.get("fault.delivery"), Some(&1));
    let empty = spans("test/fault-empty");
    assert_eq!(empty.get("fault.snapshot"), Some(&1));
    assert_eq!(empty.get("fault.delivery"), Some(&1));
    let nonowner = spans("test/fault-nonowner");
    assert_eq!(nonowner.get("fault.snapshot"), Some(&1));
    assert_eq!(nonowner.get("fault.delivery"), Some(&1));
    let no_agent = spans("test/fault-no-agent");
    assert!(!no_agent.contains_key("fault.snapshot"));
    assert!(!no_agent.contains_key("fault.delivery"));
    let throttled = spans("test/fault-throttled");
    assert!(!throttled.contains_key("fault.snapshot"));
    assert!(!throttled.contains_key("fault.delivery"));
}
