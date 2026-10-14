use super::*;
use crate::fleet::MemberKey;

const ID: &str = "note/person/avery/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9d";
const AGENT_ID: &str = "note/agent/example/reader/0192f3c4-7a1e-7c3a-9b2d-4f5e6a7b8c9e";
const AGENT: &str = "agent/example/reader";
const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

fn append(store: &Store, subject: &str, kind: &str, fields: Value) {
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: None,
        fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
        expected_subject: None, idempotency_key: None,
    }).unwrap();
}

fn store() -> Store {
    let store = Store::open_memory("alder").unwrap();
    let key = Arc::new(MemberKey::generate().unwrap().0);
    store.bind_fleet(FLEET).unwrap();
    store.pin_fleet_anchor(key.public()).unwrap();
    store.set_member_key(Some(key.clone())).unwrap();
    store.admit_fleet_anchor(FLEET, key.public(), "listening").unwrap();
    append(&store, "daemon/alder", "daemon.started", json!({"status":"running", "features":{"subject_notes":1}}));
    store
}

#[test]
fn lifecycle_preserves_creation_time_and_archive_projection_through_replay() {
    let store = store();
    let first = store.set_subject_note(ID, "person/avery", AGENT, Some("Context"), "agent", None, None).unwrap().unwrap();
    assert_eq!(first.author, "person/avery");
    assert_eq!(first.target, AGENT);
    let update = store.set_subject_note(ID, "person/avery", AGENT, Some("Changed"), "agent", None, None).unwrap().unwrap();
    assert_eq!(update.created_at, first.created_at);
    assert_ne!(update.revision, first.revision);
    assert_eq!(store.list_subject_notes(Some(AGENT), None, AGENT).unwrap(), vec![update.clone()]);
    assert_eq!(store.list_subject_notes(None, Some("person/avery"), "person/other").unwrap(), vec![update]);
    assert!(store.set_subject_note(ID, "person/avery", AGENT, None, "agent", None, None).unwrap().is_none());
    assert!(store.list_subject_notes(None, None, AGENT).unwrap().is_empty());
    let rows: usize = store.readers.get().query_row("SELECT COUNT(*) FROM subject_notes WHERE note=?1", [ID], |row| row.get(0)).unwrap();
    assert_eq!(rows, 1, "archives retain their canonical projection head");
    store.replay_replication_graph().unwrap();
    assert!(store.subject_note(ID, "person/avery").unwrap().is_none());
}

#[test]
fn self_is_person_author_only_even_for_agent_authors() {
    let store = store();
    store.set_subject_note(ID, "person/avery", AGENT, Some("Private"), "self", None, None).unwrap().unwrap();
    assert!(store.subject_note(ID, "person/avery").unwrap().is_some());
    for reader in ["person/other", AGENT] {
        assert!(store.subject_note(ID, reader).unwrap().is_none());
        assert!(store.list_subject_notes(None, None, reader).unwrap().is_empty());
    }
    assert!(store.set_subject_note(AGENT_ID, AGENT, AGENT, Some("Private agent"), "self", None, None).unwrap().is_none());
    assert!(store.subject_note(AGENT_ID, AGENT).unwrap().is_none());
    assert!(store.subject_note(AGENT_ID, "person/avery").unwrap().is_none());
    store.set_subject_note(AGENT_ID, AGENT, AGENT, Some("Shared"), "agent", None, None).unwrap().unwrap();
    assert!(store.subject_note(AGENT_ID, AGENT).unwrap().is_some());
    assert!(store.subject_note(AGENT_ID, "person/other").unwrap().is_some());
    assert!(store.subject_note(AGENT_ID, "agent/example/unrelated").unwrap().is_none());
}

#[test]
fn session_scope_requires_running_authoritative_state_not_retained_incarnation() {
    let store = store();
    append(&store, AGENT, "runtime.observed", json!({"status":"running","runtime_id":"reader","host":"alder","terminal":true,"incarnation_id":"first"}));
    assert_eq!(store.current_subject_note_incarnation(AGENT).unwrap().as_deref(), Some("first"));
    store.set_subject_note(ID, "person/avery", AGENT, Some("This session"), "agent", Some("first"), None).unwrap().unwrap();
    append(&store, AGENT, "runtime.observed", json!({"status":"exited","runtime_id":"reader","host":"alder","terminal":true,"incarnation_id":"first"}));
    assert!(store.current_subject_note_incarnation(AGENT).unwrap().is_none());
    assert!(store.subject_note(ID, AGENT).unwrap().is_none());
    append(&store, AGENT, "runtime.observed", json!({"status":"running","runtime_id":"reader","host":"alder","terminal":true,"incarnation_id":"second"}));
    assert!(store.subject_note(ID, AGENT).unwrap().is_none());
}

#[test]
fn expiry_boundaries_and_utf8_limits_are_enforced() {
    let store = store();
    let note = store.set_subject_note(ID, "person/avery", AGENT, Some(&"é".repeat(512)), "agent", None, Some("2099-01-01T00:00:00.000500Z")).unwrap().unwrap();
    let floor = st3_schema::subject_notes::expiry(note.expires_at.as_deref().unwrap()).unwrap().timestamp_millis() as u128;
    assert!(current(&store, ID, floor).unwrap().is_some());
    assert!(current(&store, ID, floor + 1).unwrap().is_none());
    for text in [" \n\t".to_owned(), "é".repeat(513)] {
        assert!(store.set_subject_note(ID, "person/avery", AGENT, Some(&text), "agent", None, None).is_err());
    }
    assert!(store.set_subject_note(ID, "person/other", AGENT, Some("Wrong author"), "agent", None, None).is_err());
    assert!(store.set_subject_note(AGENT_ID, AGENT, "agent/example/other", Some("Wrong target"), "agent", None, None).is_err());
}

#[test]
fn publication_requires_subject_notes_feature() {
    let store = Store::open_memory("alder").unwrap();
    assert_eq!(store.set_subject_note(ID, "person/avery", AGENT, Some("Context"), "agent", None, None).unwrap_err().code, "subject-note-feature-barrier");
    let store = self::store();
    append(&store, "daemon/alder", "daemon.started", json!({"status":"running", "features":{"subject_notes":0}}));
    assert_eq!(store.set_subject_note(ID, "person/avery", AGENT, Some("Context"), "agent", None, None).unwrap_err().code, "subject-note-feature-barrier");
}

#[test]
fn dirty_head_update_delete_and_rebuild_select_the_same_revision() {
    let store = store();
    let first = store.set_subject_note(ID, "person/avery", AGENT, Some("First"), "agent", None, None).unwrap().unwrap();
    let second = store.set_subject_note(ID, "person/avery", AGENT, Some("Second"), "agent", None, None).unwrap().unwrap();
    store.connection.batched(|tx| -> Result<()> {
        tx.execute("UPDATE claims SET accepted_at_unix_ms=accepted_at_unix_ms+100000 WHERE id=?1", [&first.revision])?;
        flush(tx)?;
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(store.subject_note(ID, AGENT).unwrap().unwrap().revision, first.revision);
    store.connection.batched(|tx| -> Result<()> {
        tx.execute("DELETE FROM claims WHERE id=?1", [&first.revision])?;
        flush(tx)?;
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(store.subject_note(ID, AGENT).unwrap().unwrap().revision, second.revision);
    store.connection.batched(|tx| -> Result<()> { rebuild(tx) }).unwrap().unwrap();
    assert_eq!(store.subject_note(ID, AGENT).unwrap().unwrap().revision, second.revision);
}

#[test]
fn mission_recipients_follow_current_step_assignee_and_lease_owner() {
    let store = store();
    let source = r#"version 2
agent "example/reader" { workspace "/tmp"; command "true"; }
agent "example/other" { workspace "/tmp"; command "true"; }
mission "notes" state="ready" {
  goal "Read context."
  step "work" { assigned-to "agent/example/reader"; goal "Work."; }
}
"#;
    let intent = crate::graph::parse_internal_intent(source, &store.origin).unwrap();
    let plan = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply_as(&intent, &plan.subject_tokens, "declare-notes", Some("person/avery")).unwrap();
    let mission = intent.missions["notes"].id.clone();
    let run = store.create_mission_run(&MissionRunRequest {
        mission: mission.clone(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/avery".into()), mode: Some("run".into()), inputs: BTreeMap::new(),
        idempotency_key: "notes-run".into(),
    }).unwrap();
    let step = &run.steps[0].subject;
    for target in [format!("mission/{mission}"), run.subject.clone(), step.clone()] {
        assert_eq!(store.subject_note_agents(&target).unwrap(), BTreeSet::from([AGENT.to_owned()]));
    }
    store.set_subject_note(ID, "person/avery", &run.subject, Some("Run context"), "agent", None, None).unwrap().unwrap();
    assert!(store.subject_note(ID, AGENT).unwrap().is_some());
    store.connection.batched(|tx| -> Result<()> {
        tx.execute("UPDATE step_runs SET assignee='agent/example/other',lease_owner=?2 WHERE subject=?1", params![step, AGENT])?;
        Ok(())
    }).unwrap().unwrap();
    assert_eq!(store.subject_note_agents(&run.subject).unwrap(), BTreeSet::from([AGENT.to_owned(), "agent/example/other".to_owned()]));
    assert!(store.subject_note(ID, "agent/example/other").unwrap().is_some());
    store.connection.batched(|tx| -> Result<()> {
        tx.execute("UPDATE step_runs SET status='completed' WHERE subject=?1", [step])?;
        Ok(())
    }).unwrap().unwrap();
    assert!(store.subject_note_agents(&run.subject).unwrap().is_empty());
    assert!(store.subject_note(ID, AGENT).unwrap().is_none());
}

#[test]
fn replicated_admission_binds_author_and_validates_note_fields() {
    let store = store();
    let note = store.set_subject_note(ID, "person/avery", AGENT, Some("Context"), "agent", None, None).unwrap().unwrap();
    let claim = store.claim_by_id(&note.revision).unwrap().unwrap();
    assert!(matches!(classify_replicated_claim_with_registry(&claim, st3_schema::registry()).unwrap(), ReplicatedClaimAdmission::Valid));
    for actor in [None, Some("person/other"), Some(AGENT)] {
        let mut forged = claim.clone();
        forged.actor = actor.map(str::to_owned);
        assert!(classify_replicated_claim_with_registry(&forged, st3_schema::registry()).is_err());
    }
    for text in [json!(""), json!("é".repeat(513)), json!(42)] {
        let mut invalid = claim.clone();
        invalid.body["fields"]["text"] = text;
        assert!(classify_replicated_claim_with_registry(&invalid, st3_schema::registry()).is_err());
    }
    let mut future = st3_schema::registry().clone();
    future.claims.remove(KIND);
    assert!(matches!(classify_replicated_claim_with_registry(&claim, &future).unwrap(), ReplicatedClaimAdmission::UnknownKind));
}
