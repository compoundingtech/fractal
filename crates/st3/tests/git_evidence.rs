//! Offline CLI contract: transporting/importing evidence needs no daemon or signing key.
use st3::evidence::{
    self, Check, Fact, Observation, Policy, Statement, Subject, TrustTier, Verdict,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn git(repo: &Path, args: &[&str], input: &[u8]) -> String {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn cli(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new(test_bin!("st3"))
        .args([
            "--endpoint",
            "unix:/no-evidence-daemon.sock",
            "--daemon-wait",
            "0",
            "evidence",
        ])
        .args(args)
        .arg("--repo")
        .arg(repo)
        .output()
        .unwrap()
}

fn success(output: std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn put_show_verify_work_offline_for_tests_review_and_explicit_degraded_tier() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path();
    git(repo, &["init", "-q"], b"");
    let policy = Policy {
        version: 1,
        risk_class: "daemon".into(),
        required_checks: BTreeSet::from(["unit-tests".into()]),
        allowed_tiers: BTreeSet::from([TrustTier::Full]),
        test_signers: BTreeSet::from(["fleet/fixture-observer".into()]),
        review_signers: BTreeSet::from(["fleet/fixture-observer".into()]),
    };
    let blob = git(
        repo,
        &["hash-object", "-w", "--stdin"],
        &serde_json::to_vec(&policy).unwrap(),
    );
    let config = git(
        repo,
        &["mktree"],
        format!("100644 blob {blob}\tevidence-policy.json\n").as_bytes(),
    );
    let tree = git(
        repo,
        &["mktree"],
        format!("040000 tree {config}\t.st\n").as_bytes(),
    );
    let base = git(repo, &["commit-tree", &tree], b"base\n");
    let head = git(repo, &["commit-tree", &tree, "-p", &base], b"head\n");
    let key = repo.join("throwaway-key");
    let generated = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(generated.status.success());
    let trusted = repo.join("allowed-signers");
    std::fs::write(
        &trusted,
        format!(
            "fleet/fixture-observer namespaces=\"git\" {}",
            std::fs::read_to_string(key.with_extension("pub")).unwrap()
        ),
    )
    .unwrap();
    let trusted = trusted.to_str().unwrap();
    let mut statement = Statement {
        statement_type: evidence::STATEMENT_TYPE.into(),
        predicate_type: evidence::PREDICATE_TYPE.into(),
        subject: vec![Subject {
            name: head.clone(),
            digest: BTreeMap::from([("gitTree".into(), tree.clone())]),
        }],
        predicate: Observation {
            schema_version: 1,
            change_id: "fixture-change".into(),
            signer: Some("fleet/fixture-observer".into()),
            tier: TrustTier::Full,
            re_attests: None,
            producer_sha256: "a".repeat(64),
            observed_at: chrono::Utc::now().to_rfc3339(),
            fact: Fact::Tests {
                inputs: BTreeMap::new(),
                runner: "fixture-runner".into(),
                checks: BTreeMap::from([(
                    "unit-tests".into(),
                    Check {
                        command: vec!["cargo".into(), "test".into()],
                        exit_code: 0,
                        output_sha256: "b".repeat(64),
                    },
                )]),
            },
        },
    };
    let tests = evidence::sign_observation(repo, &statement, &key).unwrap();
    assert_eq!(
        success(cli(
            repo,
            &["put", "--record", &tests, "--trusted-signers", trusted]
        ))["tier"],
        "full"
    );
    assert_eq!(
        success(cli(repo, &["show", "--record", &tests]))["predicate"]["observed"]["kind"],
        "tests"
    );
    assert_eq!(
        success(cli(
            repo,
            &["verify", "--record", &tests, "--trusted-signers", trusted]
        ))["signature_verified"],
        true
    );
    statement.predicate.fact = Fact::Review {
        reviewer: "agent/fixture-reviewer".into(),
        verdict: Verdict::GO,
        base_commit: base.clone(),
        files: evidence::changed_files(repo, &base, &head).unwrap(),
        tests: vec![tests.clone()],
    };
    let review = evidence::sign_observation(repo, &statement, &key).unwrap();
    success(cli(
        repo,
        &["put", "--record", &review, "--trusted-signers", trusted],
    ));
    assert_eq!(
        success(cli(repo, &["show", "--record", &review]))["predicate"]["observed"]["verdict"],
        "GO"
    );
    let gate = success(cli(
        repo,
        &[
            "verify",
            "--revision",
            &head,
            "--base",
            &base,
            "--change-id",
            "fixture-change",
            "--trusted-signers",
            trusted,
        ],
    ));
    assert_eq!(gate["review"], review);
    assert_eq!(gate["tier"], "full");
    // The target cannot lower requirements: policy comes from the accepted base.
    let mut stricter = policy;
    stricter.required_checks.insert("security-tests".into());
    let strict_blob = git(
        repo,
        &["hash-object", "-w", "--stdin"],
        &serde_json::to_vec(&stricter).unwrap(),
    );
    let strict_config = git(
        repo,
        &["mktree"],
        format!("100644 blob {strict_blob}\tevidence-policy.json\n").as_bytes(),
    );
    let strict_tree = git(
        repo,
        &["mktree"],
        format!("040000 tree {strict_config}\t.st\n").as_bytes(),
    );
    let strict_base = git(
        repo,
        &["commit-tree", &strict_tree, "-p", &head],
        b"accepted strict policy\n",
    );
    let weakened_head = git(
        repo,
        &["commit-tree", &tree, "-p", &strict_base],
        b"weaken policy\n",
    );
    let mut weakened = statement.clone();
    weakened.subject[0].name = weakened_head.clone();
    weakened.predicate.observed_at = chrono::Utc::now().to_rfc3339();
    weakened.predicate.fact = Fact::Review {
        reviewer: "agent/fixture-reviewer".into(),
        verdict: Verdict::GO,
        base_commit: strict_base.clone(),
        files: evidence::changed_files(repo, &strict_base, &weakened_head).unwrap(),
        tests: vec![tests],
    };
    let weakened_go = evidence::sign_observation(repo, &weakened, &key).unwrap();
    success(cli(
        repo,
        &[
            "put",
            "--record",
            &weakened_go,
            "--trusted-signers",
            trusted,
        ],
    ));
    let rejected = cli(
        repo,
        &[
            "verify",
            "--revision",
            &weakened_head,
            "--base",
            &strict_base,
            "--change-id",
            "fixture-change",
            "--trusted-signers",
            trusted,
        ],
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("required checks missing"));
    statement.predicate.signer = None;
    statement.predicate.tier = TrustTier::Degraded;
    let degraded = evidence::record_degraded(repo, &statement).unwrap();
    success(cli(
        repo,
        &["put", "--record", &degraded, "--trusted-signers", trusted],
    ));
    let result = success(cli(
        repo,
        &[
            "verify",
            "--record",
            &degraded,
            "--trusted-signers",
            trusted,
        ],
    ));
    assert_eq!(result["tier"], "degraded");
    assert_eq!(result["signature_verified"], false);
    // An explicitly degraded review cannot supply a GO when trusted keys disappear.
    std::fs::write(trusted, "").unwrap();
    assert!(
        !cli(
            repo,
            &[
                "verify",
                "--revision",
                &head,
                "--base",
                &base,
                "--change-id",
                "fixture-change",
                "--trusted-signers",
                trusted
            ]
        )
        .status
        .success()
    );
}
