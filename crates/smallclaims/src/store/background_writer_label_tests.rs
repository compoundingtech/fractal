//! The background jobs that take the writer after bind name their holds in the profile, with or
//! without task metrics: a claim range fold page and an operation claim digest backfill page are
//! each an operation of their own, and the writer hold is that operation's.
use super::runtime::Plain;
use super::*;
use serde_json::Value;

const FOLD: &str = "maintenance.claim_ranges";
const DIGESTS: &str = operation_claim_digests::LABEL;

fn slow_records() -> Vec<Value> {
    let directory = std::path::PathBuf::from(std::env::var_os("ST3_PROFILE_DIR").unwrap());
    std::fs::read_to_string(directory.join("slow.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn background_writer_pages_are_named_operations_in_the_profile() {
    // Profiling is process-global, so a child gets its own enabled profiler.
    const CHILD: &str = "ST3_PROFILE_BACKGROUND_WRITER_LABEL_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::background_writer_label_tests::background_writer_pages_are_named_operations_in_the_profile",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("ST3_PROFILE_DIR", directory.path())
            .env("ST3_PROFILE_SLOW_MS", "0")
            .status()
            .unwrap();
        assert!(status.success(), "isolated background writer label test failed");
        return;
    }
    crate::profile::init_from_env();

    let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
    store
        .append_claim(&ClaimInput {
            subject: "note/1".into(),
            kind: "example.note".into(),
            actor: None,
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some("invented-operation".into()),
        })
        .unwrap();
    // No operation on this thread, as on the daemon's blocking pool with task metrics off.
    store.fold_claim_ranges().unwrap();
    // The first page installs the table, the next copies what is stored.
    for _ in 0..2 {
        operation_claim_digests::advance(&store.readers, &store.connection).unwrap();
    }

    let records = slow_records();
    for label in [FOLD, DIGESTS] {
        let pages = records.iter().filter(|record| record["label"] == label).collect::<Vec<_>>();
        assert!(!pages.is_empty(), "{label} was not recorded as an operation");
        assert!(
            pages.iter().all(|page| page["writer_acquisitions"].as_u64().unwrap() >= 1),
            "{label}'s writer hold was not its own: {pages:?}"
        );
    }
    assert_eq!(records.iter().filter(|record| record["label"] == DIGESTS).count(), 2);

    // Inside the daemon's fold task (task metrics on), a page is part of that task: one profile
    // operation and one invocation in the performance meter, not one more per page.
    let invocations = || {
        crate::performance::snapshot()["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == FOLD)
            .map_or(0, |row| row["count"].as_u64().unwrap())
    };
    let before = (records.iter().filter(|record| record["label"] == FOLD).count(), invocations());
    crate::profile::task(FOLD, || {
        store.fold_claim_ranges().unwrap();
        store.fold_claim_ranges().unwrap();
    });
    let after = (slow_records().iter().filter(|record| record["label"] == FOLD).count(), invocations());
    assert_eq!(after, (before.0 + 1, before.1 + 1));
}
