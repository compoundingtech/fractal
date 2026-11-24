//! Payload observations survive cancellation before the authority retires the scope.
#![cfg(unix)]

use fractal_daemon::store::shell_resources::PayloadResult;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt as _, BufReader};

#[tokio::test]
async fn cancellation_reaps_payload_and_publishes_real_signal_without_git_or_logical_peak() {
    let directory = tempfile::tempdir().unwrap();
    let result_path = directory.path().join("outcome.json");
    let mut helper = tokio::process::Command::new(test_env!("CARGO_BIN_EXE_fractal-unstamped"))
        .arg("resource-payload").arg(&result_path).arg("--")
        .args(["/bin/sh", "-c", "printf 'ready\\n'; exec sleep 60"])
        .current_dir(directory.path())
        .env_remove("FRACTAL_RESOURCE_PARENT")
        .stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true)
        .spawn().unwrap();
    let mut output = BufReader::new(helper.stdout.take().unwrap()).lines();
    assert_eq!(tokio::time::timeout(Duration::from_secs(10), output.next_line()).await.unwrap().unwrap().as_deref(), Some("ready"));
    // Readiness comes from the payload, after the helper installed its signal listener.
    // SAFETY: this PID is our live child and SIGTERM is a valid signal.
    assert_eq!(unsafe { libc::kill(helper.id().unwrap() as i32, libc::SIGTERM) }, 0);
    let status = tokio::time::timeout(Duration::from_secs(10), helper.wait()).await.unwrap().unwrap();
    assert_eq!(status.code(), Some(128 + libc::SIGKILL));
    let result: PayloadResult = serde_json::from_slice(&std::fs::read(&result_path).unwrap()).unwrap();
    assert_eq!(result.exit_code, None);
    assert_eq!(result.exit_signal, Some(libc::SIGKILL));
    assert_eq!(result.head_start, None);
    assert_eq!(result.head_end, None);
    assert_eq!(result.clean_start, None);
    assert_eq!(result.clean_end, None);
    assert_eq!(result.memory_peak_bytes, None);
    let (head, clean): (Option<String>, Option<bool>) = serde_json::from_slice(
        &std::fs::read(result_path.with_extension("started")).unwrap()).unwrap();
    assert_eq!((head, clean), (None, None));
}

#[tokio::test]
async fn successful_payload_records_git_start_and_end_without_inventing_logical_peak() {
    let directory = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git").current_dir(directory.path())
            .args(args).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        output.stdout
    };
    git(&["init", "-q"]);
    std::fs::write(directory.path().join("tracked"), "before\n").unwrap();
    git(&["add", "tracked"]);
    git(&["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "initial"]);
    let head = String::from_utf8(git(&["rev-parse", "HEAD"])).unwrap().trim().to_owned();
    // Keep observation artifacts outside the repository so they cannot dirty the start.
    let observations = tempfile::tempdir().unwrap();
    let result_path = observations.path().join("outcome.json");
    let output = tokio::time::timeout(Duration::from_secs(10),
        tokio::process::Command::new(test_env!("CARGO_BIN_EXE_fractal-unstamped"))
            .arg("resource-payload").arg(&result_path).arg("--")
            .args(["/bin/sh", "-c", "printf 'after\\n' > tracked"])
            .current_dir(directory.path()).env_remove("FRACTAL_RESOURCE_PARENT")
            .kill_on_drop(true).output()).await.unwrap().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: PayloadResult = serde_json::from_slice(&std::fs::read(result_path).unwrap()).unwrap();
    assert_eq!(result.head_start.as_deref(), Some(head.as_str()));
    assert_eq!(result.head_end.as_deref(), Some(head.as_str()));
    assert_eq!(result.clean_start, Some(true));
    assert_eq!(result.clean_end, Some(false));
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.exit_signal, None);
    assert_eq!(result.memory_peak_bytes, None);
}
