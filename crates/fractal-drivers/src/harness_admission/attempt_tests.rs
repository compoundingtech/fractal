use super::*;
use crate::driver_diagnostic::{Observed, Support};

fn producer(root: &Path, body: &str) -> (PathBuf, PathBuf) {
    let binary = root.join("fake-producer");
    fs::write(
        &binary,
        format!("#!/bin/sh\n[ \"$1\" = --version ] || exit 99\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let adapter = root.join("adapter");
    fs::write(&adapter, "fake adapter").unwrap();
    (binary, adapter)
}

fn published(agent: &Path) -> crate::driver_diagnostic::Failure {
    let Observed::Failure(failure) =
        crate::driver_diagnostic::read(&crate::driver_diagnostic::path(agent))
    else {
        panic!("missing durable diagnostic")
    };
    failure
}

fn run(binary: &Path, adapter: &Path, cache: &Path, agent: &Path, incarnation: &str) -> Admission {
    observe_at(
        binary.to_str().unwrap(),
        Driver::Omp,
        Some(adapter),
        cache,
        incarnation,
        |admission| {
            admission.publish(&mut Publisher::new(
                agent,
                Driver::Omp,
                admission.version.clone(),
                admission.support(),
            ));
        },
    )
}

#[test]
fn timed_out_version_is_durable_before_scratch_cleanup_and_after_it() {
    let directory = tempfile::tempdir().unwrap();
    let (binary, adapter) = producer(
        directory.path(),
        "printf 'scratch=%s\\n' \"$HOME\" >&2\nprintf 'Authorization: Bearer fixture-credential\\n' >&2\nsleep 30",
    );
    let agent = directory.path().join("agent");
    fs::create_dir(&agent).unwrap();
    let cache = directory.path().join("cache");
    let mut home = PathBuf::new();
    let admission = observe_at(
        binary.to_str().unwrap(),
        Driver::Omp,
        Some(&adapter),
        &cache,
        "attempt-a",
        |admission| {
            let diagnostic = admission.attempt.as_ref().unwrap();
            home = PathBuf::from(
                diagnostic
                    .stderr_tail
                    .lines()
                    .next()
                    .unwrap()
                    .strip_prefix("scratch=")
                    .unwrap(),
            );
            assert!(
                home.is_dir(),
                "capture must be published before scratch deletion"
            );
            admission.publish(&mut Publisher::new(
                &agent,
                Driver::Omp,
                None,
                Support::Unsupported,
            ));
            assert!(published(&agent).attempt.is_some());
        },
    );
    assert!(!admission.passed());
    assert_eq!(admission.failed, Some(Reason::AdmissionIndeterminate));
    assert!(!home.exists());
    assert!(
        !cache.exists(),
        "a version timeout must not manufacture measured evidence"
    );
    let failure = published(&agent);
    let diagnostic = failure.attempt.unwrap();
    assert_eq!(diagnostic.incarnation, "attempt-a");
    assert_eq!(diagnostic.phase, "version");
    assert_eq!(diagnostic.outcome, "timeout");
    assert_eq!(diagnostic.deadline_ms, Some(5000));
    assert!(diagnostic.elapsed_ms >= 5000);
    assert_eq!(diagnostic.exit_signal, Some(libc::SIGKILL));
    assert!(
        diagnostic
            .error_chain
            .contains("admission process timed out")
    );
    assert!(!diagnostic.stderr_tail.contains("fixture-credential"));
}

#[test]
fn early_failures_keep_distinct_phases_and_same_code_successors_replace_predecessors() {
    let directory = tempfile::tempdir().unwrap();
    let agent = directory.path().join("agent");
    fs::create_dir(&agent).unwrap();
    let (binary, adapter) = producer(directory.path(), "printf 'omp/99.42.7\\n'");
    let cache = directory.path().join("cache-is-a-file");
    fs::write(&cache, "not a directory").unwrap();
    let first = run(
        &directory.path().join("missing"),
        &adapter,
        &cache,
        &agent,
        "first",
    );
    assert_eq!(first.failed, Some(Reason::AdmissionIndeterminate));
    assert_eq!(published(&agent).attempt.unwrap().phase, "identity");
    let second = run(&binary, &adapter, &cache, &agent, "second");
    assert_eq!(second.failed, Some(Reason::AdmissionIndeterminate));
    let diagnostic = published(&agent).attempt.unwrap();
    assert_eq!(diagnostic.phase, "cache");
    assert_eq!(diagnostic.incarnation, "second");
    assert!(!diagnostic.error_chain.is_empty());
    assert!(diagnostic.explanation("first").is_none());
}

#[test]
fn nonzero_version_exit_keeps_status_and_stderr_without_a_contract_child() {
    let directory = tempfile::tempdir().unwrap();
    let (binary, adapter) = producer(
        directory.path(),
        "printf 'producer refused fixture\\n' >&2\nexit 23",
    );
    let admission = observe_at(
        binary.to_str().unwrap(),
        Driver::Omp,
        Some(&adapter),
        &directory.path().join("cache"),
        "exit",
        |_| {},
    );
    assert_eq!(admission.failed, Some(Reason::VersionProbeFailed));
    let diagnostic = admission.attempt.unwrap();
    assert_eq!(diagnostic.phase, "version");
    assert_eq!(diagnostic.exit_code, Some(23));
    assert_eq!(diagnostic.stderr_tail, "producer refused fixture");
}

#[test]
fn scratch_creation_error_is_not_mislabeled_as_version_or_identity() {
    fn broken_scratch() -> Result<Scratch> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "fixture scratch refused",
        )
        .into())
    }
    let directory = tempfile::tempdir().unwrap();
    let (binary, adapter) = producer(directory.path(), "exit 99");
    let mut capture = Capture::new("scratch");
    let mut scratch = None;
    let error = admit_inner(
        binary.to_str().unwrap(),
        Driver::Omp,
        Some(&adapter),
        &directory.path().join("cache"),
        &mut capture,
        &mut scratch,
        broken_scratch,
    )
    .unwrap_err();
    capture.error(&error);
    let diagnostic = capture.finish(scratch.as_ref(), Driver::Omp);
    assert_eq!(diagnostic.phase, "scratch");
    assert!(diagnostic.error_chain.contains("fixture scratch refused"));
}

#[test]
fn bounded_redacted_error_chain_and_stderr_survive_read_and_retry() {
    let scratch = Scratch::new().unwrap();
    fs::write(
        scratch.path("version.stderr"),
        format!("{}\npassword = fixture-secret\n", "é".repeat(5000)),
    )
    .unwrap();
    let mut capture = Capture::new("bounded");
    capture.phase("version", Some(Duration::from_secs(5)));
    capture
        .error(&anyhow::anyhow!("Authorization:\nfixture-credential").context("outer error"));
    let diagnostic = capture.finish(Some(&scratch), Driver::Omp);
    assert!(diagnostic.stderr_tail.len() <= 4096);
    assert!(diagnostic.error_chain.len() <= 2048);
    let encoded = serde_json::to_vec(&diagnostic).unwrap();
    assert!(!String::from_utf8_lossy(&encoded).contains("fixture-secret"));
    assert!(!String::from_utf8_lossy(&encoded).contains("fixture-credential"));
    let decoded: AttemptDiagnostic = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        decoded.explanation("bounded"),
        diagnostic.explanation("bounded")
    );
    assert_eq!(
        decoded, diagnostic,
        "publication retries retain the original capture"
    );
}

#[test]
fn diagnostic_publication_failure_cannot_turn_a_refusal_into_a_pass() {
    let directory = tempfile::tempdir().unwrap();
    let (binary, adapter) = producer(directory.path(), "exit 17");
    let agent = directory.path().join("agent-is-a-file");
    fs::write(&agent, "not a directory").unwrap();
    let admission = run(
        &binary,
        &adapter,
        &directory.path().join("cache"),
        &agent,
        "refusal",
    );
    assert_eq!(admission.failed, Some(Reason::VersionProbeFailed));
    assert_eq!(admission.attempt.unwrap().exit_code, Some(17));
    assert!(!crate::driver_diagnostic::path(&agent).exists());
}

#[test]
fn cached_failure_is_not_presented_as_a_new_contract_child_error() {
    let directory = tempfile::tempdir().unwrap();
    let (binary, adapter) = producer(directory.path(), "printf 'omp/99.42.7\\n'");
    let installed =
        installed_identity(binary.to_str().unwrap(), Driver::Omp, Some(&adapter)).unwrap();
    let identity = Identity {
        driver: Driver::Omp,
        version: "99.42.7".into(),
        executable: installed.executable,
        executable_digest: installed.executable_digest,
        adapter_digest: installed.adapter_digest,
        probe_digest: probe_digest(),
    };
    let cache = directory.path().join("cache");
    fs::create_dir(&cache).unwrap();
    let path = cache.join(format!(
        "omp-{}.json",
        digest(&serde_json::to_vec(&identity).unwrap())
    ));
    fs::write(
        &path,
        serde_json::to_vec(&Record {
            schema: SCHEMA.into(),
            identity,
            observed_at: 1,
            measurements: checks(Driver::Omp)
                .iter()
                .map(|&check| measurement(check, false))
                .collect(),
            probe_failure: Some(Reason::AdmissionIndeterminate),
            diagnostic: Some(ProbeDiagnostic {
                phase: "old-contract".into(),
                outcome: "error".into(),
                exit_code: Some(19),
                elapsed_ms: 32,
                stderr_tail: "predecessor stderr".into(),
                detail: "predecessor error".into(),
            }),
        })
        .unwrap(),
    )
    .unwrap();
    let admission = observe_at(
        binary.to_str().unwrap(),
        Driver::Omp,
        Some(&adapter),
        &cache,
        "cached",
        |_| {},
    );
    assert_eq!(admission.failed, Some(Reason::AdmissionIndeterminate));
    assert_eq!(admission.cache_path.as_ref(), Some(&path));
    let diagnostic = admission.attempt.unwrap();
    assert_eq!(diagnostic.phase, "cache");
    assert_eq!(diagnostic.outcome, "cachedFailure");
    assert!(!diagnostic.error_chain.contains("predecessor error"));
    assert_eq!(diagnostic.exit_code, None);
}
