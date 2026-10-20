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

#[test]
fn partition_collision_replay_preserves_fenced_identity_historical_placement() {
    const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    let left = fixture();
    let right = Store::open_memory("example-right").unwrap();
    let other = "agent/example-other";
    let target = "agent/example-target";
    let arrangement = "arrangement/person/robin/019a0000-0000-7000-8000-000000000002";
    for store in [&left, &right] {
        store.bind_fleet(FLEET).unwrap();
    }
    let intent = crate::graph::parse_internal_intent(
        "version 2\nagent \"example-other\" { command \"true\"; }", "example-right",
    ).unwrap();
    right.apply_internal(&intent, "independent-declaration").unwrap();
    let sync = |from: &Store, to: &Store| {
        let exchange = from.export_replication_exchange_answering(
            FLEET,
            &to.replication_inventory().unwrap(),
            &to.replication_signature_requests().unwrap(),
        ).unwrap();
        to.receive_replication_exchange(&from.origin, FLEET, &exchange).unwrap();
        to.validate_replication_backlog().unwrap();
        to.project_replication_backlog().unwrap();
    };
    sync(&left, &right);
    sync(&right, &left);
    let placement_claim = left.append_client_claim(&ClaimInput {
        subject: arrangement.into(), kind: "arrangement.edited".into(),
        actor: Some("person/robin".into()),
        fields: serde_json::from_value(json!({
            "owner":"person/robin",
            "operations":[
                {"op":"create","name":"Work"},
                {"op":"subject.place","subject":ORIGINAL,"folder":null,"key":"a0"},
                {"op":"subject.place","subject":other,"folder":null,"key":"a1"}
            ]
        })).unwrap(),
        evidence: vec![], expected_subject: None,
        idempotency_key: Some("historical-placements".into()),
    }).unwrap();
    sync(&left, &right);
    let placements = |store: &Store| {
        let connection = store.readers.get();
        let mut statement = connection.prepare(
            "SELECT register,value,revision,winner FROM arrangement_registers WHERE subject=?1 AND register LIKE 'placement/%' ORDER BY register",
        ).unwrap();
        statement.query_map([arrangement], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, Vec<u8>>(3)?,
        ))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    let before = placements(&left);
    assert_eq!(before.len(), 2);
    let rename_to_target = |store: &Store, from: &str, key: &str| {
        store.rename_identity(IdentityKind::Agent, &RenameRequest {
            from: from.into(), to: target.into(), actor: "person/robin".into(),
            reason: "Clarify responsibility".into(), expected_declaration: None,
            expected_naming_revision: None, idempotency_key: key.into(),
        }, &[]).unwrap()
    };
    let a = rename_to_target(&left, ORIGINAL, "left-partition-rename");
    let b = rename_to_target(&right, other, "right-partition-rename");
    sync(&left, &right);
    sync(&right, &left);
    let loser = if a.claim_id < b.claim_id { other } else { ORIGINAL };
    for store in [&left, &right] {
        let error = store.resolve_identity(IdentityKind::Agent, loser).unwrap_err();
        assert_eq!(error.code, "lifecycle-unavailable");
        assert_eq!(error.details["reason"], "conflict");
        assert_eq!(placements(store), before);
        // Replay must project historical facts even though public ingress is fenced.
        store.replay_replication_graph().unwrap();
        assert_eq!(placements(store), before);
        let retained = placements(store).into_iter()
            .find(|(register, _, _, _)| register == &format!("placement/{loser}")).unwrap();
        assert_eq!(retained.2, placement_claim.id);
        assert_eq!(serde_json::from_str::<Value>(&retained.1).unwrap()["key"],
            if loser == ORIGINAL { "a0" } else { "a1" });
        let error = store.resolve_identity(IdentityKind::Agent, loser).unwrap_err();
        assert_eq!(error.details["reason"], "conflict");
    }
}

#[test]
fn alias_authored_placement_keeps_signed_identity_when_partition_collision_wins() {
    use st3_missions::store::arrangements::ArrangementsStore as _;
    const FLEET: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    let left = fixture();
    let right = Store::open_memory("example-right").unwrap();
    let naming_first = Store::open_memory("example-naming-first").unwrap();
    let collision_first = Store::open_memory("example-collision-first").unwrap();
    let other = "agent/example-other";
    let target = "agent/example-target";
    let arrangement = "arrangement/person/robin/019a0000-0000-7000-8000-000000000003";
    for store in [&left, &right, &naming_first, &collision_first] {
        store.bind_fleet(FLEET).unwrap();
    }
    let intent = crate::graph::parse_internal_intent(
        "version 2\nagent \"example-other\" { command \"true\"; }", "example-right",
    ).unwrap();
    right.apply_internal(&intent, "independent-declaration").unwrap();
    let sync = |from: &Store, to: &Store| {
        let exchange = from.export_replication_exchange_answering(
            FLEET, &to.replication_inventory().unwrap(),
            &to.replication_signature_requests().unwrap(),
        ).unwrap();
        to.receive_replication_exchange(&from.origin, FLEET, &exchange).unwrap();
        to.validate_replication_backlog().unwrap();
        to.project_replication_backlog().unwrap();
    };
    sync(&left, &right);
    sync(&right, &left);
    let rename_to_target = |store: &Store, from: &str, key: &str| {
        store.rename_identity(IdentityKind::Agent, &RenameRequest {
            from: from.into(), to: target.into(), actor: "person/robin".into(),
            reason: "Clarify responsibility".into(), expected_declaration: None,
            expected_naming_revision: None, idempotency_key: key.into(),
        }, &[]).unwrap()
    };
    let left_rename = rename_to_target(&left, ORIGINAL, "left-alias-placement");
    let right_rename = rename_to_target(&right, other, "right-alias-placement");
    // Choose the losing partition as A, so B -> T always wins without depending
    // on random signing keys or claim hashes. Neither partition sees the collision yet.
    let (author, competitor, a, b) = if left_rename.claim_id > right_rename.claim_id {
        (&left, &right, ORIGINAL, other)
    } else {
        (&right, &left, other, ORIGINAL)
    };
    let input = |operations, key: &str| ClaimInput {
        subject: arrangement.into(), kind: "arrangement.edited".into(),
        actor: Some("person/robin".into()),
        fields: serde_json::from_value(json!({"owner":"person/robin","operations":operations})).unwrap(),
        evidence: vec![], expected_subject: None, idempotency_key: Some(key.into()),
    };
    author.edit_arrangement(&input(json!([{"op":"create","name":"Work"}]), "alias-layout"), &BTreeMap::new()).unwrap();
    let authored = input(json!([{"op":"subject.place","subject":target,"folder":null,"key":"a1"}]), "alias-placement");
    let placement = author.edit_arrangement(&authored, &BTreeMap::new()).unwrap();
    assert_eq!(authored.fields["operations"][0]["subject"], target);
    assert_eq!(placement.body["fields"]["operations"][0]["subject"], a);
    // The normalized operation, not its authored spelling, is the retry identity.
    let mut immutable = authored.clone();
    immutable.fields.get_mut("operations").unwrap()[0]["subject"] = json!(a);
    assert_eq!(author.edit_arrangement(&immutable, &BTreeMap::new()).unwrap().id, placement.id);
    let registers = |store: &Store| {
        let connection = store.readers.get();
        let mut statement = connection.prepare(
            "SELECT register,value,revision,winner FROM arrangement_registers WHERE subject=?1 AND register LIKE 'placement/%' ORDER BY register",
        ).unwrap();
        statement.query_map([arrangement], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, Vec<u8>>(3)?,
        ))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    let expected = registers(author);
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].0, format!("placement/{a}"));
    assert_eq!(expected[0].2, placement.id);
    sync(author, &naming_first);
    sync(competitor, &naming_first);
    sync(competitor, &collision_first);
    sync(author, &collision_first);
    let signature = author.claim_signature(&placement.id).unwrap().unwrap();
    // Explicit signatures bypass normalization: refuse an alias rather than change
    // the signed operation and silently sign a different placement.
    let mut externally_signed = authored.clone();
    externally_signed.idempotency_key = Some("externally-signed-alias".into());
    let encoded_signature = serde_json::to_string(&signature).unwrap();
    let error = append_claim_with_commit_context(
        &author.graph, &externally_signed, None, None, None, None,
        ClaimCommitContext { signature: Some(&encoded_signature), ..Default::default() },
    ).unwrap_err();
    assert_eq!(error.code, "invalid-arrangement-operations");
    assert_eq!(registers(author), expected);
    for store in [&naming_first, &collision_first] {
        assert_eq!(store.resolve_identity(IdentityKind::Agent, target).unwrap().identity, b);
        assert_eq!(store.resolve_identity(IdentityKind::Agent, a).unwrap_err().details["reason"], "conflict");
        assert_eq!(registers(store), expected);
        store.replay_replication_graph().unwrap();
        assert_eq!(registers(store), expected);
        let retained = store.claim_by_id(&placement.id).unwrap().unwrap();
        assert_eq!(retained.body, placement.body);
        assert_eq!(retained.body["fields"]["operations"][0]["subject"], a);
        assert_eq!(store.claim_signature(&placement.id).unwrap().unwrap(), signature);
    }
}
