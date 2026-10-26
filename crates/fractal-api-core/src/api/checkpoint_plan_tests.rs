use super::*;
use smallclaims::store::checkpoint::hold_checkpoint_proof_lock_for_test;

#[tokio::test]
async fn checkpoint_plan_refuses_retryably_while_a_proof_holds_the_lock_for_the_whole_wait() {
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::state(root.path());
    let proof = hold_checkpoint_proof_lock_for_test(&root.path().join("checkpoint")).unwrap();
    let mut refusals = 0;
    let error = replication::plan_waiting_for_proof(
        &state, client_now_ms() + 1_000, Duration::from_millis(600), || refusals += 1,
    ).await.unwrap_err();
    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.code, "checkpoint-proof-in-progress");
    assert_eq!(error.message, "a checkpoint proof is running; retry shortly");
    assert!(refusals >= 2, "the plan polled the held lock, {refusals} retries");
    drop(proof);
}

#[tokio::test]
async fn checkpoint_plan_proceeds_once_the_running_proof_releases_its_lock() {
    let root = tempfile::tempdir().unwrap();
    let state = super::tests::state(root.path());
    let mut proof = Some(hold_checkpoint_proof_lock_for_test(&root.path().join("checkpoint")).unwrap());
    let mut refusals = 0;
    // The wait loop's first refusal releases the lock, as the running proof finishing would.
    let plan = replication::plan_waiting_for_proof(
        &state, client_now_ms() + 1_000, Duration::from_secs(60), || {
            refusals += 1;
            drop(proof.take());
        },
    ).await.unwrap();
    assert_eq!(refusals, 1);
    assert!(plan.proof.passed);
}
