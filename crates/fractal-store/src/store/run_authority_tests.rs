use super::run_authority::{covers, intersect, run_authority_mode, set_run_authority_mode};
use super::tests_shared_seats::publish_mission;
use super::*;
use crate::config::RunAuthorityMode;
use crate::model::{AuthorityGrant, AuthorityVerb, MissionRunRequest};

/// The mode is the daemon's, process-wide: tests that depend on it take turns.
static MODE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn grant(verb: AuthorityVerb, pattern: &str) -> AuthorityGrant {
    AuthorityGrant { verb, pattern: pattern.into() }
}

#[test]
fn covering_and_intersection_are_exact_for_namespaces() {
    assert!(covers("custom/factory/*", "custom/factory/landing"));
    assert!(covers("custom/factory/*", "custom/factory/deep/*"));
    assert!(covers("custom/factory/landing", "custom/factory/landing"));
    assert!(!covers("custom/factory/*", "custom/factory"));
    assert!(!covers("custom/factory/*", "custom/factoryx/landing"));
    assert!(!covers("custom/factory/landing", "custom/factory/*"));
    let declared = [grant(AuthorityVerb::Write, "custom/*"), grant(AuthorityVerb::Start, "check")];
    let starter = [
        grant(AuthorityVerb::Write, "custom/factory/*"),
        grant(AuthorityVerb::Start, "other"),
        grant(AuthorityVerb::Publish, "check"),
    ];
    assert_eq!(intersect(&declared, &starter), [grant(AuthorityVerb::Write, "custom/factory/*")]);
    assert_eq!(intersect(&starter, &declared), [grant(AuthorityVerb::Write, "custom/factory/*")]);
}

fn start(store: &Store, mission: &str, key: &str, started_by: Option<&MissionRunView>) -> Result<MissionRunView, FractalError> {
    let child = started_by.map(|parent| ChildMissionContext {
        root_revision: parent.root_revision.clone(),
        root_run_id: parent.root_mission_run.trim_start_matches("mission-run/").into(),
        parent_step_run: parent.steps[0].subject.clone(),
        default_selector: None,
        started_by: Some(parent.subject.clone()),
    });
    store.create_mission_run_inner(&MissionRunRequest {
        mission: mission.into(),
        revision: None,
        workspace: "/tmp".into(),
        requester: Some("person/operator".into()),
        mode: None,
        inputs: Default::default(),
        idempotency_key: key.into(),
    }, child, None, None)
}

fn write(store: &Store, actor: &str, subject: &str) -> Result<ClaimRecord, FractalError> {
    store.append_claim(&ClaimInput {
        subject: subject.into(),
        kind: "custom.test.observed".into(),
        actor: Some(actor.into()),
        fields: BTreeMap::from([("value".into(), json!(1))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    })
}

fn code(result: Result<impl std::fmt::Debug, FractalError>) -> String {
    result.unwrap_err().code.to_string()
}

fn missions() -> Store {
    let store = Store::open_memory("node").unwrap();
    publish_mission(&store, r#"version 2
mission "land" state="ready" {
  goal "Land one change."
  authority { write "custom/factory/*"; start "check"; }
  step "program" { agentless }
}"#, "land");
    publish_mission(&store, r#"version 2
mission "check" state="ready" {
  goal "Check it."
  authority { write "custom/*"; }
  step "program" { agentless }
}"#, "check");
    publish_mission(&store, r#"version 2
mission "other" state="ready" {
  goal "Something else."
  step "work" { agentless }
}"#, "other");
    store
}

#[test]
fn a_run_writes_only_within_its_authority_intersected_with_its_starters() {
    let _mode = MODE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    set_run_authority_mode(RunAuthorityMode::Enforce);
    let store = missions();
    let land = start(&store, "land", "land-run", None).unwrap();

    // Inside the declared authority: written as the run.
    let inside = write(&store, &land.subject, "custom/factory/landing").unwrap();
    assert_eq!(inside.actor.as_deref(), Some(land.subject.as_str()));
    // Outside it: refused, and nothing is written.
    let before = store.index().unwrap();
    assert_eq!(code(write(&store, &land.subject, "custom/other/landing")), "run-authority-denied");
    assert_eq!(store.index().unwrap(), before);

    // A program starts a child it may start; the child's creation is written as the parent run.
    let check = start(&store, "check", "check-run", Some(&land)).unwrap();
    let created = store.claims_for(&check.subject, Some("mission-run.created")).unwrap();
    assert_eq!(created[0].actor.as_deref(), Some(land.subject.as_str()));
    assert_eq!(check.requester, "person/operator");
    // The child declares `custom/*`, but its starter holds only `custom/factory/*`.
    write(&store, &check.subject, "custom/factory/check").unwrap();
    assert_eq!(code(write(&store, &check.subject, "custom/other/check")), "run-authority-denied");
    // A mission it may not start does not start.
    assert_eq!(code(start(&store, "other", "other-run", Some(&land))), "run-authority-denied");
    // A run whose mission declares nothing writes nothing.
    let other = start(&store, "other", "other-direct", None).unwrap();
    assert_eq!(code(write(&store, &other.subject, "custom/factory/other")), "run-authority-denied");

    // The person who started the run is still held to the rules that restrict them.
    store.set_rule("locked", &smallclaims::rules::Rule {
        mode: smallclaims::rules::Mode::Enforce,
        actors: vec!["person/operator".into()],
        kinds: vec!["custom.test.observed".into()],
        subjects: vec!["custom/factory/locked".into()],
        ..Default::default()
    }, "person/owner").unwrap();
    assert_eq!(code(write(&store, &land.subject, "custom/factory/locked")), "rule-denied");
    assert_eq!(code(write(&store, &check.subject, "custom/factory/locked")), "rule-denied");

    // In audit mode the write proceeds and is recorded under `rule/run-authority`.
    set_run_authority_mode(RunAuthorityMode::Audit);
    let audited = write(&store, &land.subject, "custom/other/audited").unwrap();
    assert_eq!(audited.actor.as_deref(), Some(land.subject.as_str()));
    let audits = store.rule_audits(Some("run-authority"), 10).unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].body["fields"]["actor"], land.subject);
    assert_eq!(audits[0].body["fields"]["action"], "custom.test.observed");
    assert_eq!(audits[0].body["fields"]["target"], "custom/other/audited");
    // A write inside the authority leaves no audit.
    write(&store, &land.subject, "custom/factory/again").unwrap();
    assert_eq!(store.rule_audits(Some("run-authority"), 10).unwrap().len(), 1);
    assert_eq!(run_authority_mode(), RunAuthorityMode::Audit);
    set_run_authority_mode(RunAuthorityMode::Enforce);
}

#[test]
fn writers_other_than_runs_are_untouched() {
    let _mode = MODE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Enforced by default, before any config sets the mode.
    assert_eq!(run_authority_mode(), RunAuthorityMode::Enforce);
    let store = missions();
    write(&store, "person/operator", "custom/other/person").unwrap();
    store.append_claim(&ClaimInput {
        subject: "custom/other/node".into(),
        kind: "custom.test.observed".into(),
        actor: None,
        fields: BTreeMap::from([("value".into(), json!(1))]),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    }).unwrap();
    assert_eq!(run_authority_mode(), RunAuthorityMode::Enforce);
}
