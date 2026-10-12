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

#[cfg(unix)]
#[test]
fn protected_ci_cli_fetches_selectors_and_exports_verified_objects_without_a_key() {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let caller = root.path().join("caller");
    let private = root.path().join("profile");
    let fixtures = root.path().join("api");
    let tools = root.path().join("installed-tools");
    for directory in [&caller, &private, &fixtures, &tools] {
        std::fs::create_dir(directory).unwrap();
    }
    git(&caller, &["init", "-q"], b"");
    let blob = git(
        &caller,
        &["hash-object", "-w", "--stdin"],
        b"checked source",
    );
    let recipe = b"fixture-ci-source-recipe";
    let recipe_digest = hex::encode(Sha256::digest(recipe));
    let recipe_blob = git(&caller, &["hash-object", "-w", "--stdin"], recipe);
    let scripts = git(
        &caller,
        &["mktree"],
        format!("100644 blob {recipe_blob}\tci-evidence-source\n").as_bytes(),
    );
    let tree = git(
        &caller,
        &["mktree"],
        format!("100644 blob {blob}\tcheck.rs\n040000 tree {scripts}\tscripts\n").as_bytes(),
    );
    let source = git(&caller, &["commit-tree", &tree], b"synthetic checkout");
    let scratch = private.join("observer.git");
    git(
        &private,
        &["init", "-q", "--bare", scratch.to_str().unwrap()],
        b"",
    );
    let key = private.join("throwaway-key");
    assert!(
        Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .status()
            .unwrap()
            .success()
    );
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let allowed = root.path().join("allowed-signers");
    std::fs::write(
        &allowed,
        format!("fleet/fixture-observer namespaces=\"git\" {public}"),
    )
    .unwrap();
    let object_list = format!("{source}\n{tree}\n{blob}\n{scripts}\n{recipe_blob}\n");
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&caller)
        .args(["pack-objects", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(object_list.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let pack = output.stdout;
    std::fs::write(fixtures.join("source.pack"), &pack).unwrap();
    let receipt = serde_json::json!({
        "version":1, "repository":"example/project", "event":"pull_request",
        "run_id":"123", "run_attempt":"2", "source":source, "tree":tree,
        "object_format":"sha1", "change_id":"github:example/project#7",
        "producer_sha256":recipe_digest, "pack_sha256":hex::encode(Sha256::digest(&pack)),
        "pack_bytes":pack.len()
    });
    std::fs::write(
        fixtures.join("receipt.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    assert!(Command::new("python3").args(["-c",
        "import pathlib,sys,zipfile; p=pathlib.Path(sys.argv[1]); z=zipfile.ZipFile(p/'artifact.zip','w'); [z.write(p/n,n) for n in ('receipt.json','source.pack')]; z.close()"])
        .arg(&fixtures).status().unwrap().success());
    let zip = std::fs::read(fixtures.join("artifact.zip")).unwrap();
    let metadata = serde_json::json!({
        "id":456,"expired":false,"name":"ci-evidence-source-2","workflow_run":{"id":123},
        "size_in_bytes":zip.len(),"digest":format!("sha256:{}",hex::encode(Sha256::digest(&zip)))
    });
    let run = serde_json::json!({
        "id":123,"run_attempt":2,"repository":{"full_name":"example/project"},
        "name":"Workspace CI","path":".github/workflows/fleet.yml","status":"completed",
        "event":"pull_request","head_sha":"b".repeat(40),"pull_requests":[{"number":7}]
    });
    let jobs = serde_json::json!({"jobs":[
        {"id":10,"run_id":123,"run_attempt":2,"name":"evidence-source","status":"completed",
            "conclusion":"success","steps":[
                {"name":"Retain effective tested source","status":"completed","conclusion":"success"},
                {"name":"Upload effective tested source","status":"completed","conclusion":"success"}]},
        {"id":11,"run_id":123,"run_attempt":2,"name":"unit-tests","status":"completed",
            "conclusion":"success","completed_at":"2026-10-12T12:00:00Z"}]});
    for (name, value) in [
        ("run.json", run),
        ("jobs.json", jobs),
        ("metadata.json", metadata),
    ] {
        std::fs::write(fixtures.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    std::fs::create_dir_all(private.join(".config/st")).unwrap();
    std::fs::write(
        private.join(".config/st/evidence-ci.json"),
        serde_json::to_vec(&serde_json::json!({
            "repository":"example/project","scratch_repo":scratch,
            "recipe_sha256":recipe_digest,"required_checks":["unit-tests"],
            "tier":"full",
            "signer":"fleet/fixture-observer","ssh_key":key
        }))
        .unwrap(),
    )
    .unwrap();
    let gh = tools.join("gh");
    std::fs::write(
        &gh,
        r#"#!/usr/bin/env python3
import os, pathlib, sys
assert sys.argv[1:4] == ['api', '--hostname', 'github.com']
suffix = sys.argv[4].removeprefix('repos/example/project/')
files = {'actions/runs/123/attempts/2':'run.json',
         'actions/runs/123/attempts/2/jobs?per_page=100&page=1':'jobs.json',
         'actions/artifacts/456':'metadata.json',
         'actions/artifacts/456/zip':'artifact.zip'}
sys.stdout.buffer.write((pathlib.Path(os.environ['FIXTURE_API']) / files[suffix]).read_bytes())
"#,
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let observer = |extra: &[&str]| {
        let mut paths = vec![tools.clone()];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        Command::new(test_bin!("st3"))
            .args([
                "--endpoint",
                "unix:/absent.sock",
                "--daemon-wait",
                "0",
                "evidence",
                "observe-ci",
                "--run",
                "123",
                "--attempt",
                "2",
                "--artifact",
                "456",
            ])
            .args(extra)
            .current_dir(&caller)
            .env("HOME", &private)
            .env("FIXTURE_API", &fixtures)
            .env("PATH", std::env::join_paths(paths).unwrap())
            .output()
            .unwrap()
    };
    assert!(!observer(&["--statement", "forged.json"]).status.success());
    assert!(
        !observer(&["--ssh-key", key.to_str().unwrap()])
            .status
            .success()
    );
    let result = success(observer(&[]));
    assert_eq!(result["source"], source);
    assert_eq!(result["records"][0]["tier"], "full");
    assert!(result["records"][0]["newly_recorded"].as_bool().unwrap());
    let exported = base64::engine::general_purpose::STANDARD
        .decode(result["object_pack_base64"].as_str().unwrap())
        .unwrap();
    assert!(
        !exported
            .windows(b"OPENSSH PRIVATE KEY".len())
            .any(|bytes| bytes == b"OPENSSH PRIVATE KEY")
    );
    let mut importer = Command::new("git")
        .arg("-C")
        .arg(&caller)
        .args(["index-pack", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    importer.stdin.take().unwrap().write_all(&exported).unwrap();
    assert!(importer.wait().unwrap().success());
    let record = result["records"][0]["record"].as_str().unwrap();
    let verified = evidence::verify_record(&caller, record, &allowed).unwrap();
    assert_eq!(verified.subject[0].name, source);
    evidence::put(&caller, record, &allowed).unwrap();
    assert!(
        !success(observer(&[]))["records"][0]["newly_recorded"]
            .as_bool()
            .unwrap()
    );
    let mut next_job: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("jobs.json")).unwrap()).unwrap();
    next_job["jobs"][1]["id"] = serde_json::json!(12);
    std::fs::write(
        fixtures.join("jobs.json"),
        serde_json::to_vec(&next_job).unwrap(),
    )
    .unwrap();
    let moved_key = private.join("offline-throwaway-key");
    std::fs::rename(&key, &moved_key).unwrap();
    let degraded = success(observer(&[]));
    assert_eq!(degraded["records"][0]["tier"], "degraded");
    assert_eq!(degraded["metrics"]["fresh_records"], 1);
    assert_eq!(degraded["metrics"]["full_records"], 0);
    assert_eq!(degraded["metrics"]["full_trust_share"], 0.0);
    std::fs::rename(&moved_key, &key).unwrap();
    let replay = success(observer(&[]));
    assert_eq!(
        replay["records"][0]["record"],
        degraded["records"][0]["record"]
    );
    assert_eq!(replay["records"][0]["tier"], "degraded");
    assert_eq!(replay["metrics"]["fresh_records"], 0);
    assert!(replay["metrics"]["full_trust_share"].is_null());
    // A source ZIP with a changed byte is refused before a signing observation is produced.
    let mut damaged = zip;
    damaged[10] ^= 1;
    std::fs::write(fixtures.join("artifact.zip"), damaged).unwrap();
    assert!(!observer(&[]).status.success());
}
