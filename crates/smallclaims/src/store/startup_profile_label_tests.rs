//! The first replication snapshot of a process walks every envelope identity, and the first full
//! sealing pass walks every batch. Whoever asks first, those walks are named in the profile: as
//! their own operations when the caller has none, and as spans of the caller's operation
//! otherwise. An exchange's snapshot does not walk the batches; the daemon's deferred pass does.
use super::runtime::Plain;
use super::*;
use serde_json::Value;

const FULL: &str = "replication.inventory_full";
const SEALING: &str = "replication.sealing_recovery";

fn store_with_claims(name: &str) -> Store {
    let store = Store::open_memory(name, Arc::new(Plain)).unwrap();
    for index in 0..3 {
        store
            .append_claim(&ClaimInput {
                subject: format!("note/{index}"),
                kind: "example.note".into(),
                actor: None,
                fields: BTreeMap::new(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
    store
}

fn slow_records() -> Vec<Value> {
    let directory = std::path::PathBuf::from(std::env::var_os("ST3_PROFILE_DIR").unwrap());
    std::fs::read_to_string(directory.join("slow.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn span_count(record: &Value, name: &str) -> u64 {
    record["spans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|span| span["name"] == name)
        .map_or(0, |span| span["count"].as_u64().unwrap())
}

#[test]
fn first_replication_snapshot_names_its_full_walks_in_the_profile() {
    // Profiling is process-global, so a child gets its own enabled profiler.
    const CHILD: &str = "ST3_PROFILE_STARTUP_LABEL_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::startup_profile_label_tests::first_replication_snapshot_names_its_full_walks_in_the_profile",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("ST3_PROFILE_DIR", directory.path())
            .env("ST3_PROFILE_SLOW_MS", "0")
            .status()
            .unwrap();
        assert!(status.success(), "isolated startup label test failed");
        return;
    }
    crate::profile::init_from_env();

    // No operation on this thread: each walk is an operation of its own.
    let unowned = store_with_claims("alder");
    unowned.replication_snapshot().unwrap();
    unowned.seal_local_batches().unwrap();
    let records = slow_records();
    for label in [FULL, SEALING] {
        assert!(
            records.iter().any(|record| record["label"] == label),
            "{label} was not recorded as an operation"
        );
    }

    // Inside a caller's operation: the walks are spans of it, not operations of their own.
    let owned = store_with_claims("birch");
    let before = slow_records().len();
    let caller = "POST /v1/internal/replication/export";
    let op = crate::profile::Op::start(caller, None).unwrap();
    {
        let _entered = op.enter();
        owned.replication_snapshot().unwrap();
    }
    op.finish();
    let added = slow_records().split_off(before);
    let record = added
        .iter()
        .find(|record| record["label"] == caller)
        .expect("the caller's operation was not recorded");
    assert_eq!(span_count(record, FULL), 1);
    assert_eq!(span_count(record, SEALING), 0, "an export waited for the batch walk");
    assert!(added.iter().all(|record| record["label"] != FULL && record["label"] != SEALING));

    // The daemon's deferred pass names its batch walk as a span of its own operation.
    let before = slow_records().len();
    let pass = "replication.sealing_recovery";
    let op = crate::profile::Op::start(pass, None).unwrap();
    {
        let _entered = op.enter();
        owned.seal_local_batches().unwrap();
    }
    op.finish();
    let added = slow_records().split_off(before);
    let record = added
        .iter()
        .find(|record| record["label"] == pass)
        .expect("the deferred pass was not recorded");
    assert_eq!(span_count(record, SEALING), 1);
}
