//! The trim's fixed phase spans: present only under a profiled task, and the trim does the same
//! work with or without them.
use super::*;
use crate::store::checkpoint_agreement::CheckpointAction;
use crate::store::runtime::Plain;
use std::sync::Arc;

const CHECKPOINT: &str = "checkpoint/2026-01-01";

/// A store with one checkpoint left `trimming`, as a restart after its tombstones finds it.
fn interrupted_trim() -> Store {
    let store = Store::open_memory("alder", Arc::new(Plain)).unwrap();
    store.set_checkpoint_state(CHECKPOINT, 1, "trimming").unwrap();
    store
}

fn resumed(store: &Store) -> Vec<CheckpointAction> {
    let mut actions = Vec::new();
    assert!(store.apply_stable_checkpoints(&[], std::path::Path::new("unused-checkpoint-scratch"), &mut actions).unwrap().is_none());
    actions
}

fn trimmed() -> Vec<CheckpointAction> {
    vec![CheckpointAction::Trimmed { checkpoint: CHECKPOINT.into(), envelopes: 0, claims: 0 }]
}

#[test]
fn without_a_profiled_task_there_is_no_span_and_the_trim_is_unchanged() {
    assert!(crate::profile::span("trim.delete").is_none());
    let store = interrupted_trim();
    assert_eq!(resumed(&store), trimmed());
    // Resumed once: the checkpoint is trimmed, so a second pass has nothing to resume.
    assert!(resumed(&store).is_empty());
}

#[test]
fn a_profiled_resume_records_the_resume_delete_and_final_mark_phases_once() {
    // Profiling is process-global. A child gives this test its own enabled profiler without
    // changing the accounting of other tests running in parallel.
    const CHILD: &str = "ST3_PROFILE_TRIM_PHASE_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::checkpoint_trim::phase_span_tests::a_profiled_resume_records_the_resume_delete_and_final_mark_phases_once",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("ST3_PROFILE_DIR", directory.path())
            .env("ST3_PROFILE_SLOW_MS", "0")
            .status()
            .unwrap();
        assert!(status.success(), "isolated trim phase span test failed");
        return;
    }
    crate::profile::init_from_env();
    let store = interrupted_trim();
    let label = "task checkpoint";
    let op = crate::profile::Op::start(label, None).unwrap();
    {
        let _entered = op.enter();
        assert_eq!(resumed(&store), trimmed());
    }
    drop(op);
    let directory = std::path::PathBuf::from(std::env::var_os("ST3_PROFILE_DIR").unwrap());
    let lines = std::fs::read_to_string(directory.join("slow.jsonl")).unwrap();
    let record = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|record| record["label"] == label)
        .expect("the profiled task was not recorded");
    let span = |name: &str| {
        record["spans"]
            .as_array()
            .unwrap()
            .iter()
            .find(|span| span["name"] == name)
            .cloned()
    };
    for name in ["trim.resume", "trim.delete", "trim.final_mark"] {
        assert_eq!(span(name).unwrap_or_else(|| panic!("{name} missing"))["count"], 1, "{name}");
    }
    // The final mark is two statements. This checks that the span reports their SQL; that the spans add no SQL
    // holds by construction and is not tested here.
    assert!(span("trim.final_mark").unwrap()["sql_count"].as_u64().unwrap() >= 2);
    // Nothing in a resume records tombstones or captures a plan.
    assert!(span("trim.record").is_none());
    assert!(span("trim.capture_plan").is_none());
}
