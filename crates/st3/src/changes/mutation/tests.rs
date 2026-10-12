use super::*;
use crate::model::{ClaimInput, MissionRunRequest, WorkRequest};
use serde_json::json;
use std::collections::BTreeMap;

const ACTOR: &str = "agent/fixture/worker";
const INCARNATION: &str = "fixture:one";

fn runtime(store: &Store, incarnation: &str) {
    runtime_for(store, ACTOR, incarnation);
}

fn runtime_for(store: &Store, actor: &str, incarnation: &str) {
    for (kind, fields) in [
        (
            "runtime.observed",
            BTreeMap::from([
                ("status".into(), json!("running")),
                ("incarnation_id".into(), json!(incarnation)),
            ]),
        ),
        (
            "harness.observed",
            BTreeMap::from([
                ("state".into(), json!("working")),
                ("driver".into(), json!("claude")),
                ("incarnation_id".into(), json!(incarnation)),
            ]),
        ),
    ] {
        store
            .append_claim(&ClaimInput {
                subject: actor.into(),
                kind: kind.into(),
                actor: Some(actor.into()),
                fields,
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }
}

fn work_fixture() -> (Arc<Store>, WorkBinding) {
    let store = Arc::new(Store::open_memory("fixture-owner").unwrap());
    let intent = crate::graph::parse_test_intent(
        r#"version 2
mission "fixture-stage" state="ready" {
  goal "Exercise a bounded stage."
  step "build" { assigned-to "agent/fixture/worker"; goal "Build the source." }
  step "other" { assigned-to "agent/fixture/other"; goal "Exercise another live worker." }
}
"#,
        "fixture-owner",
    )
    .unwrap();
    store.apply_internal(&intent, "fixture-stage").unwrap();
    let run = store
        .create_mission_run(&MissionRunRequest {
            mission: "fixture-stage".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/fixture".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "fixture-run".into(),
        })
        .unwrap();
    let step = run.steps[0].subject.clone();
    store.set_step_state(&step, "ready", None).unwrap();
    runtime(&store, INCARNATION);
    store
        .work_action(
            &step,
            "claim",
            &WorkRequest {
                actor: Some(ACTOR.into()),
                incarnation: Some(INCARNATION.into()),
                summary: None,
                reason: None,
                evidence: vec![],
                idempotency_key: "fixture-claim".into(),
            },
        )
        .unwrap();
    let binding = store
        .git_change_work_binding(ACTOR, &step, INCARNATION)
        .unwrap();
    (store, binding)
}

#[test]
fn work_authority_sql_is_history_independent() {
    use smallclaims::sqlite::work;
    let (store, binding) = work_fixture();
    let mut samples = Vec::new();
    for ceiling in [100, 1000] {
        for n in if ceiling == 100 { 0..100 } else { 100..1000 } {
            store.append_claim(&ClaimInput {
                subject: format!("message/authority-cost-{n}"), kind: "message.sent".into(),
                actor: Some("agent/fixture/unrelated".into()),
                fields: BTreeMap::from([
                    ("from".into(), json!("agent/fixture/unrelated")),
                    ("to".into(), json!("agent/fixture/reader")),
                    ("content".into(), json!("Unrelated authority history")),
                    ("status".into(), json!("sent")),
                ]),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        store.git_change_work_binding(ACTOR, &binding.step, INCARNATION).unwrap();
        let before = work::total();
        let selected = store.git_change_work_binding(ACTOR, &binding.step, INCARNATION).unwrap();
        store.validate_git_change_work(ACTOR, &selected).unwrap();
        assert_eq!(selected.claim_id, binding.claim_id);
        samples.push(work::total() - before);
    }
    eprintln!("authority SQL: {:?} -> {:?}", samples[0], samples[1]);
    assert_eq!(samples[0].vm_steps, samples[1].vm_steps);
    assert_eq!(samples[0].statements, samples[1].statements);
    assert_eq!(samples[1].fullscan_steps, 0);
    assert_eq!(samples[1].sorts, 0);
}

#[test]
fn work_authority_checks_exact_claim_actor_attempt_and_live_incarnation() {
    let (store, binding) = work_fixture();
    store.validate_git_change_work(ACTOR, &binding).unwrap();
    assert!(
        store
            .validate_git_change_work("agent/fixture/other", &binding)
            .is_err()
    );
    let mut wrong = binding.clone();
    wrong.claim_id = "f".repeat(64);
    assert!(store.validate_git_change_work(ACTOR, &wrong).is_err());
    wrong = binding.clone();
    wrong.incarnation = "fixture:other".into();
    assert!(store.validate_git_change_work(ACTOR, &wrong).is_err());
    store
        .work_action(
            &binding.step,
            "renew",
            &WorkRequest {
                actor: Some(ACTOR.into()),
                incarnation: Some(INCARNATION.into()),
                summary: None,
                reason: None,
                evidence: vec![],
                idempotency_key: "fixture-renew".into(),
            },
        )
        .unwrap();
    // A local quiet renewal remains the authoritative expiry and original claim.
    store.validate_git_change_work(ACTOR, &binding).unwrap();
    runtime(&store, "fixture:two");
    assert!(store.validate_git_change_work(ACTOR, &binding).is_err());
}

#[test]
fn expired_work_and_landing_lanes_cannot_authorize_publication() {
    let (store, binding) = work_fixture();
    crate::store::expire_work_lease_by_claim(&store, &binding.step, ACTOR, INCARNATION);
    assert!(store.validate_git_change_work(ACTOR, &binding).is_err());
    assert!(
        Operation::State {
            id: format!("I{}", "a".repeat(40)),
            expected_current: "b".repeat(40),
            state: State::Landed,
            next: None
        }
        .validate()
        .is_err()
    );
    assert!(
        Operation::Claim {
            id: format!("I{}", "a".repeat(40)),
            expected_current: "b".repeat(40),
            lane: "land".into()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn request_rejects_identity_selectors_unknown_fields_and_partial_object_ids() {
    let request = Request {
        repository: "fixture/garden".into(),
        work: WorkBinding {
            step: "step-run/fixture/build".into(),
            claim_id: "a".repeat(64),
            incarnation: INCARNATION.into(),
        },
        operation: Operation::Create {
            source: "b".repeat(40),
            base: "c".repeat(40),
        },
    };
    let value = serde_json::to_value(&request).unwrap();
    serde_json::from_value::<Request>(value.clone())
        .unwrap()
        .operation
        .validate()
        .unwrap();
    let mut actor = value.clone();
    actor["actor"] = json!(ACTOR);
    assert!(serde_json::from_value::<Request>(actor).is_err());
    let mut unknown = value;
    unknown["action"]["credential"] = json!("fixture-token");
    assert!(serde_json::from_value::<Request>(unknown).is_err());
    assert!(
        Operation::Create {
            source: "1234abc".into(),
            base: "b".repeat(40)
        }
        .validate()
        .is_err()
    );
}

#[test]
fn pilot_expiry_and_mutation_opt_in_are_not_derived_from_git_metadata() {
    let configuration = super::super::config::Config {
        enabled: true,
        repositories: vec![super::super::config::Repository {
            name: "fixture/garden".into(),
            reader_node: "fixture-owner".into(),
            transport_directory: "/missing/fixture/transport".into(),
            remote: "origin".into(),
            remote_url: "https://example.invalid/fixture/garden.git".into(),
            pilot_started_at: "2026-01-01T00:00:00Z".into(),
            ..Default::default()
        }],
    };
    let registry = super::super::service::Registry::new(&configuration, "fixture-other").unwrap();
    assert!(pilot_started(&registry.entry("fixture/garden").unwrap()).is_err());
    let mut enabled = configuration;
    enabled.repositories[0].mutations_enabled = true;
    let registry = super::super::service::Registry::new(&enabled, "fixture-other").unwrap();
    assert!(pilot_started(&registry.entry("fixture/garden").unwrap()).is_err());
    enabled.repositories[0].pilot_started_at = chrono::Utc::now().to_rfc3339();
    let registry = super::super::service::Registry::new(&enabled, "fixture-other").unwrap();
    pilot_started(&registry.entry("fixture/garden").unwrap()).unwrap();
}

struct GitFixture {
    _root: tempfile::TempDir,
    repository: super::super::transport::Repository,
    configuration: super::super::config::Config,
    source: String,
    base: String,
    id: String,
}

impl GitFixture {
    fn new() -> Self {
        use std::process::Command;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source");
        let remote = root.path().join("remote.git");
        for directory in [&path, &remote] {
            std::fs::create_dir(directory).unwrap();
        }
        let git = |directory: &std::path::Path, args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(directory)
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgSign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&path, &["init", "-q"]);
        git(&remote, &["init", "--bare", "-q"]);
        git(
            &path,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&path, &["commit", "--allow-empty", "-qm", "Synthetic base"]);
        let base = git(&path, &["rev-parse", "HEAD"]);
        let id = super::super::new_change_id().unwrap();
        git(
            &path,
            &[
                "commit",
                "--allow-empty",
                "-qm",
                &format!("Synthetic change\n\nChange-Id: {id}"),
            ],
        );
        let source = git(&path, &["rev-parse", "HEAD"]);
        let repository =
            super::super::transport::Repository::open(&path, Default::default()).unwrap();
        let configuration = super::super::config::Config {
            enabled: true,
            repositories: vec![super::super::config::Repository {
                name: "fixture/garden".into(),
                reader_node: "fixture-owner".into(),
                transport_directory: path,
                remote: "origin".into(),
                remote_url: remote.to_str().unwrap().into(),
                mutations_enabled: true,
                pilot_started_at: chrono::Utc::now().to_rfc3339(),
                ..Default::default()
            }],
        };
        Self {
            _root: root,
            repository,
            configuration,
            source,
            base,
            id,
        }
    }
    fn entry(&self) -> Arc<Entry> {
        super::super::service::Registry::new(&self.configuration, "fixture-owner")
            .unwrap()
            .entry("fixture/garden")
            .unwrap()
    }
    fn request(&self, work: &WorkBinding, operation: Operation) -> Request {
        Request {
            repository: "fixture/garden".into(),
            work: work.clone(),
            operation,
        }
    }
}

#[tokio::test]
async fn real_git_creation_markers_concurrent_claim_and_release_keep_exact_work_and_tree() {
    let fixture = GitFixture::new();
    let (store, work) = work_fixture();
    let entry = fixture.entry();
    let created = entry
        .mutate(
            store.clone(),
            ACTOR.into(),
            fixture.request(
                &work,
                Operation::Create {
                    source: fixture.source.clone(),
                    base: fixture.base.clone(),
                },
            ),
        )
        .await
        .unwrap()
        .revision;
    assert_eq!(created.record.change_id, fixture.id);
    assert_eq!(created.record.state, State::Draft);
    assert!(created.record.claim.is_none());
    // Exercise the real aggregate admission interval; no production/test bypass.
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    let ready = entry
        .mutate(
            store.clone(),
            ACTOR.into(),
            fixture.request(
                &work,
                Operation::State {
                    id: fixture.id.clone(),
                    expected_current: created.oid,
                    state: State::Ready,
                    next: Some("build".into()),
                },
            ),
        )
        .await
        .unwrap()
        .revision;
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    let request = fixture.request(
        &work,
        Operation::Claim {
            id: fixture.id.clone(),
            expected_current: ready.oid,
            lane: "build".into(),
        },
    );
    let (one, two) = tokio::join!(
        entry.mutate(store.clone(), ACTOR.into(), request.clone()),
        entry.mutate(store.clone(), ACTOR.into(), request)
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    let claimed = one.or(two).unwrap().revision;
    assert_eq!(claimed.record.claim.as_ref().unwrap().work, work);
    assert_eq!(claimed.record.source_tree, created.record.source_tree);
    super::super::transport::validate_advertisement(
        &fixture.repository.discover("origin").unwrap(),
        &claimed,
    )
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    let released = entry
        .mutate(
            store.clone(),
            ACTOR.into(),
            fixture.request(
                &work,
                Operation::Release {
                    id: fixture.id.clone(),
                    expected_current: claimed.oid.clone(),
                    nonce: claimed.record.claim.as_ref().unwrap().nonce.clone(),
                    next: Some("review".into()),
                },
            ),
        )
        .await
        .unwrap()
        .revision;
    assert!(released.record.claim.is_none());
    assert_eq!(released.record.next.as_deref(), Some("review"));
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    let draft = entry
        .mutate(
            store.clone(),
            ACTOR.into(),
            fixture.request(
                &work,
                Operation::State {
                    id: fixture.id.clone(),
                    expected_current: released.oid,
                    state: State::Draft,
                    next: None,
                },
            ),
        )
        .await
        .unwrap()
        .revision;
    assert_eq!(draft.record.state, State::Draft);
    assert!(draft.record.next.is_none());
    assert_eq!(draft.record.source_tree, created.record.source_tree);
    super::super::transport::validate_advertisement(
        &fixture.repository.discover("origin").unwrap(),
        &draft,
    )
    .unwrap();
    let diagnostics = entry.owner.as_ref().unwrap().diagnostics().await;
    assert_eq!(diagnostics.counters.capacity_reads_started, 5);
    assert_eq!(diagnostics.counters.discoveries_started, 0);
}

struct CapacityProbe {
    repository: super::super::transport::Repository,
    count: Option<usize>,
    gate: Option<(
        std::sync::Mutex<bool>,
        std::sync::Condvar,
        tokio::sync::Notify,
    )>,
}
impl super::super::discovery::Transport for CapacityProbe {
    fn discover(&self) -> Result<super::super::transport::Advertisement> {
        self.repository.discover("origin")
    }
    fn sync(&self) -> Result<()> {
        self.repository.sync("origin")
    }
    fn capacity(&self) -> Result<(super::super::transport::Advertisement, usize)> {
        if let Some((lock, condition, started)) = &self.gate {
            started.notify_one();
            let mut released = lock.lock().unwrap();
            while !*released {
                released = condition.wait(released).unwrap();
            }
        }
        let (advertisement, count) = self.repository.discover_capacity("origin")?;
        Ok((advertisement, self.count.unwrap_or(count)))
    }
}

fn with_probe(fixture: &GitFixture, probe: Arc<CapacityProbe>) -> Arc<Entry> {
    let mut entry = fixture.entry();
    Arc::get_mut(&mut entry).unwrap().owner = Some(super::super::discovery::Owner::new(probe));
    entry
}

#[tokio::test]
async fn canceled_client_keeps_publication_slot_and_publishes_one_record() {
    let fixture = GitFixture::new();
    let (store, work) = work_fixture();
    let probe = Arc::new(CapacityProbe {
        repository: fixture.repository.clone(),
        count: None,
        gate: Some((
            std::sync::Mutex::new(false),
            std::sync::Condvar::new(),
            tokio::sync::Notify::new(),
        )),
    });
    let entry = with_probe(&fixture, probe.clone());
    let request = fixture.request(
        &work,
        Operation::Create {
            source: fixture.source.clone(),
            base: fixture.base.clone(),
        },
    );
    let client = tokio::spawn({
        let entry = entry.clone();
        let store = store.clone();
        let request = request.clone();
        async move { entry.mutate(store, ACTOR.into(), request).await }
    });
    probe.gate.as_ref().unwrap().2.notified().await;
    client.abort();
    assert!(entry.publishing.try_lock().is_err());
    assert!(
        entry
            .mutate(store.clone(), ACTOR.into(), request)
            .await
            .unwrap_err()
            .to_string()
            .contains("in flight")
    );
    let (lock, condition, _) = probe.gate.as_ref().unwrap();
    *lock.lock().unwrap() = true;
    condition.notify_all();
    let completed = entry.publishing.lock().await;
    assert!(
        store
            .latest_indexed_git_change("fixture/garden", &fixture.id)
            .unwrap()
            .is_some()
    );
    drop(completed);
    assert_eq!(fixture.repository.discover("origin").unwrap().refs.len(), 3);
    assert_eq!(
        entry
            .owner
            .as_ref()
            .unwrap()
            .diagnostics()
            .await
            .counters
            .capacity_reads_started,
        1
    );
}

#[tokio::test]
async fn capacity_prior_history_and_missing_trailer_refuse_creation_without_a_push() {
    let fixture = GitFixture::new();
    let (store, work) = work_fixture();
    let probe = Arc::new(CapacityProbe {
        repository: fixture.repository.clone(),
        count: Some(5000),
        gate: None,
    });
    let entry = with_probe(&fixture, probe);
    let request = fixture.request(
        &work,
        Operation::Create {
            source: fixture.source.clone(),
            base: fixture.base.clone(),
        },
    );
    assert!(
        entry
            .mutate(store.clone(), ACTOR.into(), request.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("5000")
    );
    assert!(
        fixture
            .repository
            .discover("origin")
            .unwrap()
            .refs
            .is_empty()
    );
    let missing = fixture.request(
        &work,
        Operation::Create {
            source: fixture.base.clone(),
            base: fixture.base.clone(),
        },
    );
    assert!(
        fixture
            .entry()
            .mutate(store.clone(), ACTOR.into(), missing)
            .await
            .unwrap_err()
            .to_string()
            .contains("no Change-Id")
    );
    for number in 1..=100 {
        let mut revision = crate::store::git_changes::tests::revision();
        revision.oid = format!("{number:040x}");
        revision.record.change_id = format!("I{number:040x}");
        store.index_git_change("fixture/garden", &revision).unwrap();
    }
    assert!(
        fixture
            .entry()
            .mutate(store.clone(), ACTOR.into(), request)
            .await
            .unwrap_err()
            .to_string()
            .contains("100 changes")
    );
    assert!(
        fixture
            .repository
            .discover("origin")
            .unwrap()
            .refs
            .is_empty()
    );
}

#[tokio::test]
async fn create_refuses_historical_identity_and_incompatible_commit_range() {
    let fixture = GitFixture::new();
    let (store, work) = work_fixture();
    let mut historical = crate::store::git_changes::tests::revision();
    historical.record.change_id = fixture.id.clone();
    store
        .index_git_change("fixture/garden", &historical)
        .unwrap();
    let request = fixture.request(
        &work,
        Operation::Create {
            source: fixture.source.clone(),
            base: fixture.base.clone(),
        },
    );
    assert!(
        fixture
            .entry()
            .mutate(store.clone(), ACTOR.into(), request)
            .await
            .unwrap_err()
            .to_string()
            .contains("owner repair")
    );
    let other_id = super::super::new_change_id().unwrap();
    let directory = &fixture.configuration.repositories[0].transport_directory;
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(directory)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--allow-empty",
            "-qm",
            &format!("Another synthetic unit\n\nChange-Id: {other_id}"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let (source, _) = fixture.repository.source("HEAD").unwrap();
    let request = fixture.request(
        &work,
        Operation::Create {
            source,
            base: fixture.base.clone(),
        },
    );
    assert!(
        fixture
            .entry()
            .mutate(store.clone(), ACTOR.into(), request)
            .await
            .unwrap_err()
            .to_string()
            .contains("incompatible Change-Ids")
    );
    assert!(
        fixture
            .repository
            .discover("origin")
            .unwrap()
            .refs
            .is_empty()
    );
}

#[tokio::test]
async fn different_live_worker_cannot_change_state_and_deferred_is_typed() {
    let fixture = GitFixture::new();
    let (store, work) = work_fixture();
    let other_actor = "agent/fixture/other";
    let other_incarnation = "fixture:other";
    let other_step = format!("{}/other", work.step.strip_suffix("/build").unwrap());
    store.set_step_state(&other_step, "ready", None).unwrap();
    runtime_for(&store, other_actor, other_incarnation);
    store
        .work_action(
            &other_step,
            "claim",
            &WorkRequest {
                actor: Some(other_actor.into()),
                incarnation: Some(other_incarnation.into()),
                summary: None,
                reason: None,
                evidence: vec![],
                idempotency_key: "fixture-other-claim".into(),
            },
        )
        .unwrap();
    let other_work = store
        .git_change_work_binding(other_actor, &other_step, other_incarnation)
        .unwrap();
    let entry = fixture.entry();
    let created = entry
        .mutate(
            store.clone(),
            ACTOR.into(),
            fixture.request(
                &work,
                Operation::Create {
                    source: fixture.source.clone(),
                    base: fixture.base.clone(),
                },
            ),
        )
        .await
        .unwrap()
        .revision;
    let operation = Operation::State {
        id: fixture.id.clone(),
        expected_current: created.oid.clone(),
        state: State::Ready,
        next: Some("build".into()),
    };
    let deferred = entry
        .mutate(
            store.clone(),
            ACTOR.into(),
            fixture.request(&work, operation.clone()),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(deferred.downcast_ref::<MutationFailure>(), Some(MutationFailure::Deferred { retry_after_ms }) if *retry_after_ms > 0)
    );
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    let foreign = entry
        .mutate(
            store.clone(),
            other_actor.into(),
            fixture.request(&other_work, operation),
        )
        .await
        .unwrap_err();
    assert!(foreign.to_string().contains("current change owner"));
    super::super::transport::validate_advertisement(
        &fixture.repository.discover("origin").unwrap(),
        &created,
    )
    .unwrap();
}
