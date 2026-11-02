//! A checkpoint's capture, plan and proof phases as spans: present only under a profiled task,
//! each phase once per dry run.
use super::checkpoint_tests_shared_seats::{AGENT, input};
use super::*;

const PHASES: [&str; 10] = [
    "checkpoint.capture",
    "checkpoint.plan",
    "proof.copy",
    "proof.keep_sealed",
    "proof.replay_before",
    "proof.answers_before",
    "proof.drop",
    "proof.replay_after",
    "proof.answers_after",
    "proof.rollback",
];

/// Plan and prove a checkpoint of a seat's settled harness history, which the rules drop.
fn dry_run() -> CheckpointPlanView {
    let store = Store::open_memory("alder").unwrap();
    for (n, state) in ["idle", "working", "idle", "working", "idle"].iter().enumerate() {
        store
            .append_claim_outcome(&input(
                AGENT,
                "harness.observed",
                Some(AGENT),
                json!({"state": state, "incarnation_id": "inc-1", "observed_at_ms": n}),
                &format!("harness-{n}"),
            ))
            .unwrap();
    }
    let scratch = tempfile::tempdir().unwrap();
    let view = store.checkpoint_plan_view(now_ms() + 1_000, scratch.path()).unwrap();
    assert!(view.proof.passed, "{:?}", view.proof.mismatches);
    assert!(view.dropped_claims > 0, "the proof replays a drop");
    view
}

#[test]
fn without_a_profiled_task_there_is_no_span_and_the_proof_is_unchanged() {
    assert!(smallclaims::profile::span("proof.copy").is_none());
    dry_run();
}

#[test]
fn a_profiled_dry_run_records_each_capture_plan_and_proof_phase_once() {
    // Profiling is process-global. A child gives this test its own enabled profiler without
    // changing the accounting of other tests running in parallel.
    const CHILD: &str = "ST3_PROFILE_PROOF_PHASE_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::checkpoint_proof_span_tests::a_profiled_dry_run_records_each_capture_plan_and_proof_phase_once",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("ST3_PROFILE_DIR", directory.path())
            .env("ST3_PROFILE_SLOW_MS", "0")
            .status()
            .unwrap();
        assert!(status.success(), "isolated proof phase span test failed");
        return;
    }
    smallclaims::profile::init_from_env();
    let label = "task checkpoint";
    let op = smallclaims::profile::Op::start(label, None).unwrap();
    {
        let _entered = op.enter();
        dry_run();
    }
    drop(op);
    let directory = std::path::PathBuf::from(std::env::var_os("ST3_PROFILE_DIR").unwrap());
    let lines = std::fs::read_to_string(directory.join("slow.jsonl")).unwrap();
    let record = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|record| record["label"] == label)
        .expect("the profiled task was not recorded");
    let spans = record["spans"].as_array().unwrap();
    for name in PHASES {
        let span = spans
            .iter()
            .find(|span| span["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing: {spans:?}"));
        assert_eq!(span["count"], 1, "{name}");
        // Each phase counts its thread's reads, so a proof's disk shows by phase.
        assert!(span.get("disk_read_mb").is_some(), "{name}: {span}");
    }
}
