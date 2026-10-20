//! Rename changes addressing, never physical custody or authority identity.
use super::*;
use crate::DeliveryStore as _;
use st3_store::store::identities::{IdentityKind, IdentityStore as _, RenameRequest};

const ORIGINAL: &str = "agent/example-old";
const CANONICAL: &str = "agent/example-privileged/new";

fn fixture() -> Store {
    let store = Store::open_memory("fixture").unwrap();
    let intent = crate::graph::parse_internal_intent(
        "version 2\nagent \"example-old\" { command \"true\"; }\nmission \"example-work\" state=\"ready\" { goal \"Preserve custody\"; step \"work\" { assigned-to \"agent/example-old\"; goal \"Keep working\"; } }",
        "fixture",
    ).unwrap();
    store.apply_internal(&intent, "initial-declaration").unwrap();
    for (kind, fields) in [
        ("runtime.observed", json!({"status":"running","runtime_id":"example-physical","incarnation_id":"current"})),
        ("harness.observed", json!({"state":"ready","driver":"omp","incarnation_id":"current"})),
    ] {
        append(&store, ORIGINAL, kind, fields, kind);
    }
    store
}

fn append(store: &Store, subject: &str, kind: &str, fields: Value, key: &str) -> ClaimRecord {
    store.append_claim(&ClaimInput {
        subject: subject.into(), kind: kind.into(), actor: Some("person/robin".into()),
        fields: serde_json::from_value(fields).unwrap(), evidence: vec![], expected_subject: None,
        idempotency_key: Some(key.into()),
    }).unwrap()
}

fn rename(store: &Store) {
    store.rename_identity(IdentityKind::Agent, &RenameRequest {
        from: ORIGINAL.into(), to: CANONICAL.into(), actor: "person/robin".into(),
        reason: "Use a clearer public name".into(), expected_declaration: None,
        expected_naming_revision: None, idempotency_key: "rename-boundary".into(),
    }, &[]).unwrap();
}

#[test]
fn rename_keeps_mailbox_capability_and_delivers_sends_to_both_spellings() {
    let store = fixture();
    let fence = store.bind_mailbox(&crate::mailbox::Fence::new(ORIGINAL, "current", "delivery")).unwrap();
    rename(&store);
    store.check_mailbox(&fence).unwrap();
    let mut canonical_fence = fence.clone();
    canonical_fence.subject = CANONICAL.into();
    let rebound = store.bind_mailbox(&canonical_fence).unwrap();
    assert_eq!(rebound.subject, ORIGINAL);
    assert_eq!(rebound.token, fence.token);
    assert_eq!(rebound.epoch, fence.epoch);
    assert_eq!(rebound.incarnation, fence.incarnation);
    let mut stale = canonical_fence;
    stale.incarnation = "stale".into();
    assert!(store.check_mailbox(&stale).is_err());
    for (subject, recipient) in [("message/example-old-send", ORIGINAL), ("message/example-new-send", CANONICAL)] {
        append(&store, subject, "message.sent", json!({"from":"person/robin","to":recipient,"content":"Still the same mailbox"}), subject);
    }
    let old = store.messages(Some(ORIGINAL), false).unwrap();
    let new = store.messages(Some(CANONICAL), false).unwrap();
    assert_eq!(old.iter().map(|message| &message.subject).collect::<Vec<_>>(), new.iter().map(|message| &message.subject).collect::<Vec<_>>());
    assert_eq!(old.len(), 2);
    let runtime = store.latest_claim(ORIGINAL, Some("runtime.observed")).unwrap().unwrap();
    assert_eq!(runtime.body["fields"]["incarnation_id"], "current");
    assert_eq!(runtime.body["fields"]["runtime_id"], "example-physical");
}

#[test]
fn rename_retains_work_lease_and_canonical_declaration_updates_original_row() {
    let store = fixture();
    let run = store.create_mission_run(&MissionRunRequest {
        mission: "example-work".into(), revision: None, workspace: "/tmp".into(),
        requester: Some("person/robin".into()), mode: Some("run".into()), inputs: BTreeMap::new(),
        idempotency_key: "work-run".into(),
    }).unwrap();
    let subject = run.steps[0].subject.clone();
    store.connection.batched(|tx| -> Result<()> {
        let claim = append_claim_tx(tx, "fixture", &subject, "work.claimed", Some(ORIGINAL),
            &json!({"fields":{"attempt":1,"status":"claimed","claimant":ORIGINAL,"claim_incarnation":"current","claim_expires_at_unix_ms":(now_ms()+600_000) as u64}}), &[], None)?;
        project_mission_run_update(tx, &claim)?;
        Ok(())
    }).unwrap().unwrap();
    let before = store.step_run(&subject).unwrap().unwrap();
    rename(&store);
    for spelling in [ORIGINAL, CANONICAL] {
        let work = store.open_work_for_seat(spelling).unwrap();
        let current = work.iter().find(|step| step.subject == subject).unwrap();
        assert_eq!(current.run, before.run);
        assert_eq!(current.generation, before.generation);
        assert_eq!(current.attempt, before.attempt);
        assert_eq!(current.claimant, before.claimant);
        assert_eq!(current.claim_incarnation, before.claim_incarnation);
    }
    let intent = crate::graph::parse_internal_intent(
        "version 2\nagent \"example-privileged/new\" { command \"false\"; }", "fixture",
    ).unwrap();
    store.apply_internal(&intent, "canonical-redeclaration").unwrap();
    let connection = store.readers.get();
    let rows: u64 = connection.query_row("SELECT COUNT(*) FROM desired WHERE subject IN (?1,?2)", params![ORIGINAL,CANONICAL], |row| row.get(0)).unwrap();
    assert_eq!(rows, 1);
    assert!(current_desired_row(&connection, ORIGINAL).unwrap().is_some());
    assert_eq!(store.resolve_identity(IdentityKind::Agent, CANONICAL).unwrap().identity, ORIGINAL);
}

#[test]
fn rename_into_secret_grant_prefix_never_widens_authority() {
    let store = fixture();
    rename(&store);
    let resolved = store.resolve_identity(IdentityKind::Agent, CANONICAL).unwrap();
    assert!(crate::sekrets::identity::principal_matches("agent/example-privileged/**", &resolved.canonical_id));
    assert!(!crate::sekrets::identity::principal_matches("agent/example-privileged/**", &resolved.identity));
    assert!(crate::sekrets::identity::principal_matches(ORIGINAL, &resolved.identity));
    assert!(!crate::sekrets::identity::principal_matches(CANONICAL, &resolved.identity));
}

#[test]
fn sidebar_placement_keeps_order_and_register_revision_after_rename() {
    let store = fixture();
    let arrangement = "arrangement/person/robin/019a0000-0000-7000-8000-000000000001";
    let edit = |operations, key: &str| {
        let input = ClaimInput {
            subject: arrangement.into(), kind: "arrangement.edited".into(),
            actor: Some("person/robin".into()),
            fields: serde_json::from_value(json!({"owner":"person/robin","operations":operations})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: Some(key.into()),
        };
        store.append_client_claim(&input).unwrap()
    };
    edit(json!([{"op":"create","name":"Work"},
        {"op":"subject.place","subject":ORIGINAL,"folder":null,"key":"a0"}]), "placement-initial");
    let read = || crate::store::arrangements::arrangement_at(&store.readers.get(), arrangement, store.index().unwrap()).unwrap().unwrap();
    let before = read();
    let placement = before["body"]["placements"][ORIGINAL].clone();
    rename(&store);
    let after = read();
    assert_eq!(after["body"]["placements"][CANONICAL], placement);
    assert!(after["body"]["placements"].get(ORIGINAL).is_none());
    assert_eq!(after["revision"], before["revision"]);
    edit(json!([{"op":"subject.place","subject":CANONICAL,"folder":null,"key":"a1"}]), "placement-canonical-edit");
    let current = read();
    assert_eq!(current["body"]["placements"].as_object().unwrap().len(), 1);
    assert_eq!(current["body"]["placements"][CANONICAL]["value"]["key"], "a1");
}
