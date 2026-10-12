use super::*;
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
    key: PathBuf,
    trusted: PathBuf,
    base: String,
    head: String,
}

fn command(program: &str, args: &[&str]) {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn policy() -> Policy {
    Policy {
        version: 1,
        risk_class: "daemon".into(),
        required_checks: BTreeSet::from(["unit-tests".into()]),
        allowed_tiers: BTreeSet::from([TrustTier::Full]),
        test_signers: BTreeSet::from(["fleet/fixture-observer".into()]),
        review_signers: BTreeSet::from(["fleet/fixture-observer".into()]),
    }
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path();
        command("git", &["init", "-q", repo.to_str().unwrap()]);
        let key = repo.join("throwaway-key");
        command(
            "ssh-keygen",
            &["-q", "-t", "ed25519", "-N", "", "-f", key.to_str().unwrap()],
        );
        let trusted = repo.join("allowed-signers");
        let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
        std::fs::write(
            &trusted,
            format!("fleet/fixture-observer namespaces=\"git\" {public}"),
        )
        .unwrap();
        let mut fixture = Self {
            root,
            key,
            trusted,
            base: String::new(),
            head: String::new(),
        };
        fixture.base = fixture.revision("before", None);
        fixture.head = fixture.revision("after", Some(&fixture.base));
        fixture
    }

    fn repo(&self) -> &Path {
        self.root.path()
    }

    fn revision(&self, content: &str, parent: Option<&str>) -> String {
        let file = git_text(
            self.repo(),
            &["hash-object", "-w", "--stdin"],
            Some(content.as_bytes()),
        )
        .unwrap();
        let policy_blob = git_text(
            self.repo(),
            &["hash-object", "-w", "--stdin"],
            Some(&serde_json::to_vec(&policy()).unwrap()),
        )
        .unwrap();
        let policy_tree = git_text(
            self.repo(),
            &["mktree"],
            Some(format!("100644 blob {policy_blob}\tevidence-policy.json\n").as_bytes()),
        )
        .unwrap();
        let tree = git_text(
            self.repo(),
            &["mktree"],
            Some(
                format!("040000 tree {policy_tree}\t.st\n100644 blob {file}\tcheck.rs\n")
                    .as_bytes(),
            ),
        )
        .unwrap();
        self.commit_tree(&tree, parent)
    }

    fn commit_tree(&self, tree: &str, parent: Option<&str>) -> String {
        let mut args = vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit-tree",
            tree,
        ];
        if let Some(parent) = parent {
            args.extend(["-p", parent]);
        }
        git_text(self.repo(), &args, Some(b"fixture\n")).unwrap()
    }

    fn statement(&self, fact: Fact) -> Statement {
        Statement {
            statement_type: STATEMENT_TYPE.into(),
            subject: vec![Subject {
                name: self.head.clone(),
                digest: BTreeMap::from([(
                    "gitTree".into(),
                    tree(self.repo(), &self.head).unwrap(),
                )]),
            }],
            predicate_type: PREDICATE_TYPE.into(),
            predicate: Observation {
                schema_version: 1,
                change_id: "fixture-change".into(),
                signer: Some("fleet/fixture-observer".into()),
                tier: TrustTier::Full,
                re_attests: None,
                producer_sha256: "a".repeat(64),
                observed_at: chrono::Utc::now().to_rfc3339(),
                fact,
            },
        }
    }

    fn tests(&self, runner: &str, exit_code: i32) -> Statement {
        self.statement(Fact::Tests {
            inputs: BTreeMap::new(),
            runner: runner.into(),
            checks: BTreeMap::from([(
                "unit-tests".into(),
                Check {
                    command: vec!["cargo".into(), "test".into()],
                    exit_code,
                    output_sha256: "b".repeat(64),
                },
            )]),
        })
    }

    fn review(&self, tests: Vec<String>, verdict: Verdict) -> Statement {
        self.statement(Fact::Review {
            reviewer: "agent/fixture-reviewer".into(),
            verdict,
            base_commit: self.base.clone(),
            files: changed_files(self.repo(), &self.base, &self.head).unwrap(),
            tests,
        })
    }

    fn put(&self, statement: &Statement) -> String {
        let id = sign_observation(self.repo(), statement, &self.key).unwrap();
        put(self.repo(), &id, &self.trusted).unwrap();
        id
    }

    fn verify(&self, head: &str, base: &str) -> Result<Verified> {
        verify(
            self.repo(),
            head,
            base,
            "fixture-change",
            &self.trusted,
            &policy(),
        )
    }
}

#[test]
fn tests_and_review_round_trip_and_gate() {
    let f = Fixture::new();
    let tests = f.put(&f.tests("fixture-runner", 0));
    let review = f.put(&f.review(vec![tests.clone()], Verdict::GO));
    assert!(matches!(
        show(f.repo(), &tests).unwrap().predicate.fact,
        Fact::Tests { .. }
    ));
    assert!(matches!(
        verify_record(f.repo(), &review, &f.trusted)
            .unwrap()
            .predicate
            .fact,
        Fact::Review {
            verdict: Verdict::GO,
            ..
        }
    ));
    let verified = f.verify(&f.head, &f.base).unwrap();
    assert_eq!(verified.tests, vec![tests]);
    assert_eq!(verified.review, review);
    assert_eq!(load_policy(f.repo(), &f.head).unwrap().risk_class, "daemon");
}

#[test]
fn forged_signature_missing_key_and_wrong_principal_are_rejected() {
    let f = Fixture::new();
    let mut missing_signer = f.tests("fixture-runner", 0);
    missing_signer.predicate.signer = None;
    assert!(sign_observation(f.repo(), &missing_signer, &f.key).is_err());
    let tests = sign_observation(f.repo(), &f.tests("fixture-runner", 0), &f.key).unwrap();
    let raw = git(f.repo(), &["cat-file", "commit", &tests], None).unwrap();
    // Tamper with signed content while retaining the original SSH signature.
    let forged = String::from_utf8(raw)
        .unwrap()
        .replace("st gate observation v1", "forged observation");
    let id = git_text(
        f.repo(),
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        Some(forged.as_bytes()),
    )
    .unwrap();
    assert!(put(f.repo(), &id, &f.trusted).is_err());
    std::fs::write(&f.trusted, "").unwrap();
    assert!(verify_record(f.repo(), &tests, &f.trusted).is_err());
    let public = std::fs::read_to_string(f.key.with_extension("pub")).unwrap();
    std::fs::write(&f.trusted, format!("fleet/fixture-other {public}")).unwrap();
    assert!(verify_record(f.repo(), &tests, &f.trusted).is_err());
    assert!(records(f.repo()).unwrap().is_empty());
}

#[test]
fn changed_file_blob_needs_a_fresh_review() {
    let f = Fixture::new();
    let tests = f.put(&f.tests("fixture-runner", 0));
    f.put(&f.review(vec![tests], Verdict::GO));
    let changed = f.revision("changed again", Some(&f.head));
    assert!(f.verify(&changed, &f.base).is_err());
}

#[test]
fn clean_rebase_with_same_files_and_cited_tests_keeps_go() {
    let f = Fixture::new();
    let tests = f.put(&f.tests("fixture-runner", 0));
    let review = f.put(&f.review(vec![tests], Verdict::GO));
    let base_tree = tree(f.repo(), &f.base).unwrap();
    let new_base = f.commit_tree(&base_tree, Some(&f.base));
    let head_tree = tree(f.repo(), &f.head).unwrap();
    let rebased = f.commit_tree(&head_tree, Some(&new_base));
    assert_ne!(rebased, f.head);
    assert_eq!(f.verify(&rebased, &new_base).unwrap().review, review);
}

#[test]
fn missing_failed_or_changed_cited_tests_block_stage() {
    let f = Fixture::new();
    assert!(f.verify(&f.head, &f.base).is_err());
    let failed = f.put(&f.tests("fixture-runner", 1));
    f.put(&f.review(vec![failed], Verdict::GO));
    assert!(f.verify(&f.head, &f.base).is_err());
    let passing = f.put(&f.tests("fixture-runner-2", 0));
    f.put(&f.review(vec![passing.clone()], Verdict::GO));
    assert!(f.verify(&f.head, &f.base).is_ok());
    git(
        f.repo(),
        &["update-ref", "-d", &format!("{NOTES_PREFIX}{passing}")],
        None,
    )
    .unwrap();
    // A new successful test cannot silently substitute for the test a GO cites.
    f.put(&f.tests("fixture-runner-3", 0));
    assert!(f.verify(&f.head, &f.base).is_err());
}

#[test]
fn stop_wrong_change_and_unapproved_role_cannot_supply_go() {
    let f = Fixture::new();
    let tests = f.put(&f.tests("fixture-runner", 0));
    f.put(&f.review(vec![tests.clone()], Verdict::STOP));
    assert!(f.verify(&f.head, &f.base).is_err());
    let mut review = f.review(vec![tests.clone()], Verdict::GO);
    review.predicate.change_id = "fixture-other-change".into();
    f.put(&review);
    assert!(f.verify(&f.head, &f.base).is_err());
    f.put(&f.review(vec![tests], Verdict::GO));
    let mut p = policy();
    p.review_signers = BTreeSet::from(["fleet/fixture-other".into()]);
    assert!(verify(f.repo(), &f.head, &f.base, "fixture-change", &f.trusted, &p).is_err());
}

#[test]
fn partial_review_and_extra_current_paths_are_rejected() {
    let f = Fixture::new();
    let tests = f.put(&f.tests("fixture-runner", 0));
    let mut partial = f.review(vec![tests.clone()], Verdict::GO);
    if let Fact::Review { files, .. } = &mut partial.predicate.fact {
        files.clear();
    }
    assert!(sign_observation(f.repo(), &partial, &f.key).is_err());
    f.put(&f.review(vec![tests], Verdict::GO));
    // The wrong base makes additional paths part of the change: cannot reuse GO.
    let empty = git_text(f.repo(), &["mktree"], Some(b"")).unwrap();
    let other_base = f.commit_tree(&empty, None);
    assert!(f.verify(&f.head, &other_base).is_err());
}

#[test]
fn simultaneous_writers_keep_every_record_and_same_import_is_idempotent() {
    let f = Fixture::new();
    let ids: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..6)
            .map(|i| {
                let f = &f;
                scope.spawn(move || f.put(&f.tests(&format!("fixture-runner-{i}"), 0)))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    assert_eq!(records(f.repo()).unwrap().len(), 6);
    git(
        f.repo(),
        &["update-ref", "-d", &format!("{NOTES_PREFIX}{}", ids[0])],
        None,
    )
    .unwrap();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..6)
            .map(|_| scope.spawn(|| put(f.repo(), &ids[0], &f.trusted).unwrap()))
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    assert_eq!(records(f.repo()).unwrap().len(), 6);
}

#[test]
fn notes_push_and_fetch_carry_source_and_work_without_shared_daemon() {
    let f = Fixture::new();
    let tests = f.put(&f.tests("fixture-runner", 0));
    f.put(&f.review(vec![tests], Verdict::GO));
    let remote = tempfile::tempdir().unwrap();
    command(
        "git",
        &["init", "--bare", "-q", remote.path().to_str().unwrap()],
    );
    git(
        f.repo(),
        &[
            "push",
            remote.path().to_str().unwrap(),
            "refs/notes/evidence/*:refs/notes/evidence/*",
        ],
        None,
    )
    .unwrap();
    let peer = tempfile::tempdir().unwrap();
    command("git", &["init", "-q", peer.path().to_str().unwrap()]);
    git(
        peer.path(),
        &[
            "fetch",
            remote.path().to_str().unwrap(),
            "refs/notes/evidence/*:refs/notes/evidence/*",
        ],
        None,
    )
    .unwrap();
    assert!(
        verify(
            peer.path(),
            &f.head,
            &f.base,
            "fixture-change",
            &f.trusted,
            &load_policy(peer.path(), &f.head).unwrap()
        )
        .is_ok()
    );
}

#[test]
fn untrusted_repo_signature_config_and_replace_objects_do_not_grant_trust() {
    let f = Fixture::new();
    let tests = f.put(&f.tests("fixture-runner", 0));
    let fake = f.repo().join("fake-signature-program");
    std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
    git(
        f.repo(),
        &["config", "gpg.ssh.program", fake.to_str().unwrap()],
        None,
    )
    .unwrap();
    git(
        f.repo(),
        &[
            "config",
            "gpg.ssh.allowedSignersFile",
            f.trusted.to_str().unwrap(),
        ],
        None,
    )
    .unwrap();
    let no_keys = f.repo().join("no-keys");
    std::fs::write(&no_keys, "").unwrap();
    assert!(verify_record(f.repo(), &tests, &no_keys).is_err());
    let other = f.put(&f.tests("fixture-other-runner", 0));
    git(f.repo(), &["replace", &tests, &other], None).unwrap();
    let statement = verify_record(f.repo(), &tests, &f.trusted).unwrap();
    assert!(
        matches!(statement.predicate.fact, Fact::Tests { runner, .. } if runner == "fixture-runner")
    );
}

#[test]
fn policy_must_have_nonempty_checks_and_signer_roles() {
    let mut p = policy();
    p.required_checks.clear();
    assert!(p.validate().is_err());
    p = policy();
    p.test_signers.clear();
    assert!(p.validate().is_err());
    p = policy();
    p.risk_class = "unknown".into();
    assert!(p.validate().is_err());
}

#[test]
fn newer_stop_supersedes_go_and_a_second_fleet_key_can_be_added() {
    let f = Fixture::new();
    let tests = f.put(&f.tests("fixture-runner", 0));
    f.put(&f.review(vec![tests.clone()], Verdict::GO));
    assert!(f.verify(&f.head, &f.base).is_ok());
    f.put(&f.review(vec![tests.clone()], Verdict::STOP));
    assert!(f.verify(&f.head, &f.base).is_err());
    // A second throwaway key from the same fixture fleet stands in for a future peer.
    let peer_key = f.repo().join("throwaway-peer-key");
    command(
        "ssh-keygen",
        &[
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-f",
            peer_key.to_str().unwrap(),
        ],
    );
    let mut peer_go = f.review(vec![tests], Verdict::GO);
    peer_go.predicate.signer = Some("fleet/fixture-peer".into());
    let id = sign_observation(f.repo(), &peer_go, &peer_key).unwrap();
    assert!(put(f.repo(), &id, &f.trusted).is_err());
    use std::io::Write;
    let public = std::fs::read_to_string(peer_key.with_extension("pub")).unwrap();
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&f.trusted)
            .unwrap(),
        "fleet/fixture-peer namespaces=\"git\" {public}"
    )
    .unwrap();
    put(f.repo(), &id, &f.trusted).unwrap();
    let mut p = policy();
    // Cryptographic trust alone must not widen the allowed review role.
    assert!(f.verify(&f.head, &f.base).is_err());
    p.review_signers.insert("fleet/fixture-peer".into());
    assert_eq!(
        verify(f.repo(), &f.head, &f.base, "fixture-change", &f.trusted, &p)
            .unwrap()
            .review,
        id
    );
    // Removing a peer key after a mismatch immediately revokes its prior evidence.
    let public = std::fs::read_to_string(f.key.with_extension("pub")).unwrap();
    std::fs::write(
        &f.trusted,
        format!("fleet/fixture-observer namespaces=\"git\" {public}"),
    )
    .unwrap();
    assert!(verify_record(f.repo(), &id, &f.trusted).is_err());
    assert!(verify(f.repo(), &f.head, &f.base, "fixture-change", &f.trusted, &p).is_err());
}

#[test]
fn degraded_and_test_tiers_are_explicit_and_never_satisfy_daemon_policy() {
    let f = Fixture::new();
    let mut degraded = f.tests("fixture-runner", 0);
    degraded.predicate.tier = TrustTier::Degraded;
    degraded.predicate.signer = None;
    assert!(sign_observation(f.repo(), &degraded, &f.key).is_err());
    let id = record_degraded(f.repo(), &degraded).unwrap();
    put(f.repo(), &id, &f.trusted).unwrap();
    assert_eq!(
        verify_record(f.repo(), &id, &f.trusted)
            .unwrap()
            .predicate
            .tier,
        TrustTier::Degraded
    );
    f.put(&f.review(vec![id], Verdict::GO));
    assert!(f.verify(&f.head, &f.base).is_err());
    let mut test = f.tests("fixture-test-runner", 0);
    test.predicate.tier = TrustTier::Test;
    let id = f.put(&test);
    assert_eq!(
        verify_record(f.repo(), &id, &f.trusted)
            .unwrap()
            .predicate
            .tier,
        TrustTier::Test
    );
    f.put(&f.review(vec![id], Verdict::GO));
    assert!(f.verify(&f.head, &f.base).is_err());
    let mut p = policy();
    p.allowed_tiers.insert(TrustTier::Test);
    assert!(p.validate().is_err());
    p.allowed_tiers = BTreeSet::from([TrustTier::Degraded]);
    assert!(p.validate().is_err());
}

#[test]
fn fresh_full_rerun_re_attests_without_promoting_degraded_record() {
    let f = Fixture::new();
    let mut degraded = f.tests("fixture-runner", 0);
    degraded.predicate.tier = TrustTier::Degraded;
    degraded.predicate.signer = None;
    let prior = record_degraded(f.repo(), &degraded).unwrap();
    put(f.repo(), &prior, &f.trusted).unwrap();
    let mut rerun = f.tests("fixture-rerun", 0);
    rerun.predicate.re_attests = Some(prior.clone());
    let full = f.put(&rerun);
    f.put(&f.review(vec![full], Verdict::GO));
    assert_eq!(f.verify(&f.head, &f.base).unwrap().tier, TrustTier::Full);
    assert_eq!(
        show(f.repo(), &prior).unwrap().predicate.tier,
        TrustTier::Degraded
    );
    if let Fact::Tests { checks, .. } = &mut rerun.predicate.fact {
        checks
            .get_mut("unit-tests")
            .unwrap()
            .command
            .push("--different-input".into());
    }
    let forged_rerun = sign_observation(f.repo(), &rerun, &f.key).unwrap();
    assert!(put(f.repo(), &forged_rerun, &f.trusted).is_err());
}

#[test]
fn open_test_inputs_round_trip_but_do_not_authorize_scoped_reuse() {
    let f = Fixture::new();
    let mut tests = f.tests("fixture-runner", 0);
    if let Fact::Tests { inputs, .. } = &mut tests.predicate.fact {
        inputs.insert("crate/fixture".into(), "sha256:opaque-input-key".into());
    }
    let id = f.put(&tests);
    assert!(
        matches!(show(f.repo(), &id).unwrap().predicate.fact, Fact::Tests { inputs, .. } if inputs["crate/fixture"] == "sha256:opaque-input-key")
    );
    f.put(&f.review(vec![id], Verdict::GO));
    let changed = f.revision("changed source", Some(&f.head));
    assert!(f.verify(&changed, &f.base).is_err());
    let mut unknown = tests;
    unknown.predicate.schema_version = 2;
    assert!(sign_observation(f.repo(), &unknown, &f.key).is_err());
}
