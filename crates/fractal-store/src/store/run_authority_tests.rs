use super::run_authority::{covers, intersect, run_authority_mode, set_run_authority_mode};
use super::tests_shared_seats::publish_mission;
use super::*;
use crate::config::RunAuthorityMode;
use crate::model::{AuthorityGrant, AuthorityVerb, MissionRunRequest};

/// The mode is the daemon's, process-wide: tests that depend on it take turns, and each gives
/// back the mode it found.
static MODE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct ModeGuard {
    before: RunAuthorityMode,
    _turn: std::sync::MutexGuard<'static, ()>,
}

impl Drop for ModeGuard {
    fn drop(&mut self) {
        set_run_authority_mode(self.before);
    }
}

fn hold_mode() -> ModeGuard {
    let turn = MODE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    ModeGuard { before: run_authority_mode(), _turn: turn }
}

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
  authority { write "custom/factory/*"; write "doc/factory/*"; start "check"; revise "check"; }
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

/// The grant a claim needs follows the operation it belongs to, decided by fields its writer
/// cannot choose.
#[test]
fn a_claims_needed_grant_follows_its_operation() {
    use super::run_authority::needed_tx;
    let store = missions();
    let land = start(&store, "land", "land-needs", None).unwrap();
    let check = start(&store, "check", "check-needs", None).unwrap();
    let other = start(&store, "other", "other-needs", None).unwrap();
    let run = land.subject.trim_start_matches("mission-run/");
    let generation = |view: &MissionRunView| view.steps[0].subject.split('/').nth(1).unwrap().to_owned();
    let mut connection = store.connection.write();
    let transaction = connection.transaction().unwrap();
    let needs = |kind: &str, subject: &str, fields: Value| {
        let fields = fields.as_object().unwrap().clone().into_iter().collect::<BTreeMap<_, _>>();
        needed_tx(&transaction, run, kind, subject, &fields).unwrap()
            .map(|grant| format!("{} {}", grant.verb.as_str(), grant.pattern))
    };
    let next = "run-generation/next";
    // A revision's generation names its predecessor, whatever its reason says.
    assert_eq!(needs("run-generation.created", next,
        json!({"run": check.subject, "predecessor": format!("run-generation/{}", generation(&check)), "reason": "initial mission run"})),
        Some("revise check".into()));
    assert_eq!(needs("run-generation.created", next, json!({"run": check.subject, "reason": "anything"})), Some("start check".into()));
    // What a revision writes on the runs it changes needs `revise` on their missions.
    assert_eq!(needs("step-run.carried", &check.steps[0].subject, json!({})), Some("revise check".into()));
    assert_eq!(needs("run-generation.superseded", &format!("run-generation/{}", generation(&check)), json!({})), Some("revise check".into()));
    assert_eq!(needs("step-run.state", &other.steps[0].subject, json!({})), Some("revise other".into()));
    assert_eq!(needs("mission-run.state", &other.subject, json!({})), Some("revise other".into()));
    assert_eq!(needs("revision-proposal.created", "revision-proposal/one", json!({"run": check.subject})), Some("revise check".into()));
    // A run journals its own steps freely; another run's journal is a write like any other.
    assert_eq!(needs("program.step", &land.steps[0].subject, json!({})), None);
    assert_eq!(needs("program.step", &check.steps[0].subject, json!({})), Some(format!("write {}", check.steps[0].subject)));
    assert_eq!(needs("custom.test.observed", "custom/factory/x", json!({})), Some("write custom/factory/x".into()));
}

/// A document a run binds is decided by its run authority, as its claims are.
#[test]
fn a_runs_document_writes_are_decided_by_its_authority() {
    let _mode = hold_mode();
    set_run_authority_mode(RunAuthorityMode::Enforce);
    let store = missions();
    let land = start(&store, "land", "land-docs", None).unwrap();
    let put = |name: &str| store.put_document_as(name, b"notes", &None, name, Some(&land.subject));
    put("doc/factory/notes").unwrap();
    assert_eq!(code(put("doc/elsewhere/notes")), "run-authority-denied");
    set_run_authority_mode(RunAuthorityMode::Audit);
    put("doc/elsewhere/audited").unwrap();
    let audits = store.rule_audits(Some("run-authority"), 10).unwrap();
    assert_eq!(audits[0].body["fields"]["target"], "doc/elsewhere/audited");
    assert_eq!(audits[0].body["fields"]["action"], "doc.bound");
}

#[test]
fn a_run_writes_only_within_its_authority_intersected_with_its_starters() {
    let _mode = hold_mode();
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
}

#[test]
fn writers_other_than_runs_are_untouched() {
    let _mode = hold_mode();
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

/// A run cancels another run by applying a `mission-run` declaration as itself.
fn cancel(store: &Store, actor: &MissionRunView, target: &MissionRunView, key: &str) -> Result<ApplyResponse, FractalError> {
    let source = format!(
        "version 2\nmission-run {:?} {{ cancellation {key:?} {{ reason \"the test cancels it\" }} }}\n",
        target.subject,
    );
    let intent = crate::graph::parse_intent(&source, "node").unwrap();
    let preview = store.mission(&intent, IntentInput { kdl: source.clone(), source_name: None }).unwrap();
    store.apply_as(&intent, &preview.subject_tokens, key, Some(&actor.subject))
}

/// A run touches only itself and the runs it started, transitively; a `write "mission-run/*"`
/// grant reaches no sibling, parent or unrelated run.
#[test]
fn a_run_touches_only_its_own_descendant_runs() {
    let _mode = hold_mode();
    set_run_authority_mode(RunAuthorityMode::Enforce);
    let store = missions();
    publish_mission(&store, r#"version 2
mission "steward" state="ready" {
  goal "Steward runs."
  concurrent-runs max=4
  authority { write "mission-run/*"; start "steward"; }
  step "program" { agentless }
}"#, "steward");
    let root = start(&store, "steward", "steward-root", None).unwrap();
    let child = start(&store, "steward", "steward-child", Some(&root)).unwrap();
    let sibling = start(&store, "steward", "steward-sibling", Some(&root)).unwrap();
    let grandchild = start(&store, "steward", "steward-grandchild", Some(&child)).unwrap();
    let unrelated = start(&store, "other", "steward-unrelated", None).unwrap();
    let phase = |run: &MissionRunView| store.mission_run(&run.subject).unwrap().unwrap().phase;

    for (actor, target, case) in [
        (&root, &unrelated, "unrelated"),
        (&child, &root, "parent"),
        (&child, &sibling, "sibling"),
        (&grandchild, &root, "grandparent"),
    ] {
        let before = store.index().unwrap();
        assert_eq!(code(cancel(&store, actor, target, &format!("cancel-{case}"))), "run-authority-denied", "{case}");
        assert_eq!(store.index().unwrap(), before, "{case}");
        assert_eq!(phase(target), "normal", "{case}");
    }

    // The runs it started, transitively, are its own.
    assert!(cancel(&store, &root, &grandchild, "cancel-grandchild").unwrap().changed);
    assert_ne!(phase(&grandchild), "normal");
    assert!(cancel(&store, &child, &child, "cancel-itself").unwrap().changed);
    assert_ne!(phase(&child), "normal");
    assert_eq!(phase(&root), "normal");
    assert_eq!(phase(&sibling), "normal");
    assert_eq!(phase(&unrelated), "normal");
}
