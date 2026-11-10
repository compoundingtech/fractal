use super::*;
use crate::{edits::{Base, Event, Identity, Refusal}, paging::{Scope, Snapshot}, plan::{MoveIntent, V2Plan, plan_v2}, projection::build_v2};
use fractal_schema::arrangements::{self, Register};
fn folder(id: &str) -> NodeRef { NodeRef::Folder { id: id.into() } }
fn subject(id: &str) -> NodeRef { NodeRef::Subject { subject: id.into() } }
fn parent(id: &str) -> Parent { Parent::Folder { id: id.into() } }
fn scope() -> Scope { Scope { owner: "owner-a".into(), container: "container-a".into(), endpoint: "endpoint-a".into(), host: "host-a".into(), projection: "1".into() } }
fn base(n: u64) -> Base { Base { scope: scope(), snapshot: Snapshot { id: format!("snapshot-{n}"), host_id: "host-a".into(), store_index: n, projection_version: "1".into(), created_at: "created".into(), published_at: None }, revision: format!("revision-{n}"), ready: true } }
fn identity(n: u64) -> Identity { Identity { actor: "actor-a".into(), action_id: format!("action-{n}"), idempotency_key: format!("key-{n}") } }
fn inputs() -> (ArrangementInput, Vec<MembershipInput>) {
    let folders = ["folder-a", "folder-b"].into_iter().map(|id| (id.into(), arrangements::Folder {
        name: Register { value: id.into(), revision: "r1".into() },
        position: Register { value: arrangements::Position { parent: None, key: "a1".into() }, revision: "r1".into() }, tombstone: None,
    })).collect();
    let arrangement = ArrangementInput {
        subject: "container-a".into(), owner: "owner-a".into(), revision: "r1".into(), deleted: false,
        body: arrangements::VersionedBody::V2(arrangements::BodyV2 { version: 2, name: Register { value: "Sidebar".into(), revision: "r1".into() }, folders }),
        resolved: Some(ResolvedInput { parents: [("folder-a".into(), None), ("folder-b".into(), None)].into(), folders: Default::default() }),
    };
    let members = ["member-a", "member-b"].into_iter().map(|id| MembershipInput {
        container: "container-a".into(), member: id.into(), revision: "m1".into(), position: fractal_schema::ordered_membership::Position { bucket: None, key: "a0".into() },
    }).collect();
    (arrangement, members)
}
fn observed_inputs() -> (ArrangementInput, Vec<MembershipInput>, SidebarTree) {
    let (arrangement, members) = inputs();
    let observed = build_v2(&arrangement, &members).unwrap();
    (arrangement, members, observed)
}
fn staged(intent: MoveIntent, plan: V2Plan) -> Edits {
    let mut edits = Edits::new(scope(), intent);
    edits.reduce(Event::Plan { base: base(1), plan, folder: identity(1), membership: identity(2) });
    edits
}
fn member_edit() -> (Edits, SidebarTree, SidebarTree) {
    let (arrangement, mut members, observed) = observed_inputs();
    let intent = MoveIntent { node: subject("member-a"), parent: parent("folder-a"), before: None };
    let plan = plan_v2(&build_v2(&arrangement, &members).unwrap(), &intent).unwrap();
    let Member::Place { bucket, key, .. } = &plan.memberships[0] else { panic!("move emits Place"); };
    members[0].position = fractal_schema::ordered_membership::Position { bucket: bucket.clone(), key: key.clone() };
    (staged(intent, plan), observed, build_v2(&arrangement, &members).unwrap())
}
#[test]
fn member_move_is_visible_next_frame_without_mutating_observation() {
    let (edits, observed, expected) = member_edit();
    assert_eq!(edits.status, Status::AwaitingPersistence);
    assert_eq!(edits.project(&observed).unwrap().as_ref(), &expected);
    assert!(observed.nodes["folder:folder-a"].children.is_empty());
    assert_eq!((&expected.nodes["folder:folder-a"].children, &expected.roots), (&vec![subject("member-a")], &vec![folder("folder-a"), folder("folder-b"), subject("member-b")]));
}
#[test]
fn folder_move_and_folder_first_row_order_use_real_planner() {
    let (arrangement, members, observed) = observed_inputs();
    let intent = MoveIntent { node: folder("folder-b"), parent: parent("folder-a"), before: None };
    let edits = staged(intent.clone(), plan_v2(&build_v2(&arrangement, &members).unwrap(), &intent).unwrap());
    let visible = edits.project(&observed).unwrap();
    assert_eq!((&visible.nodes["folder:folder-a"].children, &visible.roots, &visible.nodes["folder:folder-b"].shown_under), (&vec![folder("folder-b")], &vec![folder("folder-a"), subject("member-a"), subject("member-b")], &parent("folder-a")));
}
#[test]
fn compound_plan_displays_only_staged_folder_step_until_fresh_membership_plan() {
    let (mut arrangement, members, observed) = observed_inputs();
    let intent = MoveIntent { node: subject("member-a"), parent: parent("folder-a"), before: None };
    let member_plan = plan_v2(&build_v2(&arrangement, &members).unwrap(), &intent).unwrap().memberships;
    let mut edits = staged(intent, V2Plan { folders: vec![Folder::FolderMove { id: "folder-b".into(), parent: Some("folder-a".into()), key: "a0".into() }], memberships: member_plan.clone() });
    let visible = edits.project(&observed).unwrap();
    assert_eq!(visible.nodes["folder:folder-a"].children, vec![folder("folder-b")]);
    assert_eq!(visible.nodes["subject:member-a"].shown_under, Parent::Root {});
    // Authoritative folder readback arrives before completion persistence/fresh planning.
    let arrangements::VersionedBody::V2(body) = &mut arrangement.body else { unreachable!() };
    body.folders.get_mut("folder-b").unwrap().position.value.parent = Some("folder-a".into());
    body.folders.get_mut("folder-b").unwrap().position.value.key = "a0".into();
    arrangement.resolved.as_mut().unwrap().parents.insert("folder-b".into(), Some("folder-a".into()));
    let confirmed = build_v2(&arrangement, &members).unwrap();
    edits.reduce(Event::Persisted(identity(1))); edits.reduce(Event::Observed(identity(1)));
    assert!(matches!(edits.project(&confirmed).unwrap(), Cow::Borrowed(_)));
    edits.reduce(Event::CompletionPersisted(identity(1)));
    assert_eq!(edits.project(&confirmed).unwrap().as_ref(), &confirmed);
    edits.reduce(Event::Fresh(base(2), Ok(member_plan)));
    let visible = edits.project(&confirmed).unwrap();
    assert_eq!(visible.nodes["folder:folder-a"].children, vec![folder("folder-b"), subject("member-a")]);
    // Refusing step two rolls back only its rows, never the completed folder step.
    edits.reduce(Event::Persisted(identity(2))); edits.reduce(Event::Refused(identity(2), Refusal::Unauthorized));
    assert_eq!(edits.status, Status::PartiallyApplied);
    assert_eq!(edits.project(&confirmed).unwrap().as_ref(), &confirmed);
}
#[test]
fn refusal_restores_observed_rows_and_retry_does_not_reapply_old_request() {
    let (mut edits, observed, expected) = member_edit();
    assert_eq!(edits.project(&observed).unwrap().as_ref(), &expected);
    edits.reduce(Event::Persisted(identity(2))); edits.reduce(Event::Refused(identity(2), Refusal::StaleFence));
    assert_eq!(edits.project(&observed).unwrap().as_ref(), &observed);
    assert!(edits.intent().is_some());
    edits.reduce(Event::Retry);
    assert!(matches!(edits.project(&observed).unwrap(), Cow::Borrowed(_)));
}
#[test]
fn unknown_delivery_and_ack_keep_visible_overlay_until_observed() {
    let (mut edits, observed, expected) = member_edit();
    edits.reduce(Event::Persisted(identity(2))); edits.reduce(Event::Unknown(identity(2)));
    assert_eq!(edits.project(&observed).unwrap().as_ref(), &expected);
    edits.reduce(Event::Replay);
    assert_eq!(edits.project(&observed).unwrap().as_ref(), &expected);
    edits.reduce(Event::Ack(identity(2)));
    assert_eq!(edits.project(&observed).unwrap().as_ref(), &expected);
}
#[test]
fn observation_drops_overlay_before_completion_persistence_without_double_apply() {
    let (mut edits, observed, confirmed) = member_edit();
    let before = edits.project(&observed).unwrap().into_owned();
    edits.reduce(Event::Persisted(identity(2))); edits.reduce(Event::Observed(identity(2)));
    assert_eq!(before, confirmed);
    assert!(matches!(edits.project(&confirmed).unwrap(), Cow::Borrowed(_)));
    // A newer concurrent winner must not be overwritten by a completed request.
    assert_eq!(edits.project(&observed).unwrap().as_ref(), &observed);
    edits.reduce(Event::CompletionPersisted(identity(2)));
    assert_eq!(edits.project(&confirmed).unwrap().as_ref(), &before);
}
#[test]
fn folder_create_delete_lifts_children_and_remove_changes_visible_rows() {
    let (arrangement, mut members) = inputs();
    for member in &mut members { member.position.bucket = Some("folder-a".into()); }
    let observed = build_v2(&arrangement, &members).unwrap();
    let intent = MoveIntent { node: folder("folder-b"), parent: parent("folder-a"), before: None };
    let mut edits = staged(intent, V2Plan { folders: vec![
        Folder::FolderCreate { id: "folder-c".into(), name: "C".into(), parent: Some("folder-a".into()), key: "a0".into() },
        Folder::FolderMove { id: "folder-b".into(), parent: Some("folder-c".into()), key: "a0".into() },
        Folder::FolderRename { id: "folder-c".into(), name: "Renamed".into() },
        Folder::FolderDelete { id: "folder-c".into() },
    ], memberships: vec![] });
    let visible = edits.project(&observed).unwrap();
    assert_eq!((&visible.nodes["folder:folder-a"].children, visible.nodes.contains_key("folder:folder-c")), (&vec![folder("folder-b"), subject("member-a"), subject("member-b")], false));
    edits.reduce(Event::Persisted(identity(1))); edits.reduce(Event::Refused(identity(1), Refusal::Conflict));
    assert_eq!(edits.project(&observed).unwrap().as_ref(), &observed);
    let intent = MoveIntent { node: subject("member-a"), parent: Parent::Root {}, before: None };
    let edits = staged(intent, V2Plan { folders: vec![], memberships: vec![Member::Remove { member: "member-a".into() }] });
    assert!(!edits.project(&observed).unwrap().nodes.contains_key("subject:member-a"));
}
#[test]
fn atomic_delete_move_lifts_from_final_positions_in_every_operation_order() {
    let (mut arrangement, mut members) = inputs();
    let arrangements::VersionedBody::V2(body) = &mut arrangement.body else { unreachable!() };
    let template = body.folders["folder-a"].clone();
    for id in ["folder-c", "folder-d"] { body.folders.insert(id.into(), template.clone()); }
    body.folders.get_mut("folder-b").unwrap().position.value.parent = Some("folder-a".into());
    body.folders.get_mut("folder-c").unwrap().position.value.parent = Some("folder-b".into());
    let resolved = arrangement.resolved.as_mut().unwrap();
    resolved.parents.insert("folder-b".into(), Some("folder-a".into()));
    resolved.parents.insert("folder-c".into(), Some("folder-b".into()));
    resolved.parents.insert("folder-d".into(), None);
    members[0].position.bucket = Some("folder-b".into());
    members[1].position.bucket = Some("folder-c".into());
    let observed = build_v2(&arrangement, &members).unwrap();
    let operations = [
        Folder::FolderDelete { id: "folder-a".into() },
        Folder::FolderMove { id: "folder-b".into(), parent: Some("folder-d".into()), key: "a0".into() },
        Folder::FolderDelete { id: "folder-b".into() },
    ];
    let arrangements::VersionedBody::V2(body) = &mut arrangement.body else { unreachable!() };
    for id in ["folder-a", "folder-b"] { body.folders.get_mut(id).unwrap().tombstone = Some(Register { value: true, revision: "r2".into() }); }
    body.folders.get_mut("folder-b").unwrap().position.value.parent = Some("folder-d".into());
    arrangement.resolved.as_mut().unwrap().parents.insert("folder-c".into(), Some("folder-d".into()));
    members[0].position.bucket = Some("folder-d".into());
    let expected = build_v2(&arrangement, &members).unwrap();
    for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        let intent = MoveIntent { node: folder("folder-b"), parent: parent("folder-d"), before: None };
        let edits = staged(intent, V2Plan { folders: order.map(|index| operations[index].clone()).into(), memberships: vec![] });
        let visible = edits.project(&observed).unwrap();
        assert_eq!(visible.as_ref(), &expected, "order {order:?}");
        assert_eq!(visible.nodes["folder:folder-d"].children, vec![folder("folder-c"), subject("member-a")]);
        assert_eq!(visible.nodes["folder:folder-c"].children, vec![subject("member-b")]);
    }
}
#[test]
fn create_delete_preserves_tombstone_and_missing_ancestor_rules_in_both_orders() {
    let (arrangement, mut members) = inputs();
    for member in &mut members { member.position.bucket = Some("folder-a".into()); }
    let observed = build_v2(&arrangement, &members).unwrap();
    let create = Folder::FolderCreate { id: "folder-c".into(), name: "C".into(), parent: Some("missing".into()), key: "a0".into() }; let delete = Folder::FolderDelete { id: "folder-c".into() };
    for folders in [vec![delete.clone(), create.clone()], vec![create, delete]] {
        let intent = MoveIntent { node: folder("folder-b"), parent: parent("folder-c"), before: None }; let mut folders = folders;
        folders.push(Folder::FolderMove { id: "folder-b".into(), parent: Some("folder-c".into()), key: "a0".into() });
        let visible = staged(intent.clone(), V2Plan { folders, memberships: vec![] }).project(&observed).unwrap().into_owned();
        assert!(!visible.nodes.contains_key("folder:folder-c")); assert_eq!(visible.deleted_folders, vec!["folder-c"]);
        assert_eq!(visible.nodes["folder:folder-b"].shown_under, Parent::Root {}); assert_eq!(visible.nodes["folder:folder-b"].parent, parent("folder-c"));
        assert!(matches!(crate::plan::plan_move(&visible, &intent), Err(crate::plan::MoveRefusal::ParentMissing { cause: crate::plan::ParentMissingCause::FolderDeleted, .. })));
    }
    // C was tombstoned under live A, itself nested in B; SidebarTree omits C's position.
    let mut retained = arrangement.clone(); let arrangements::VersionedBody::V2(body) = &mut retained.body else { unreachable!() };
    let mut dead = body.folders["folder-a"].clone(); dead.position.value.parent = Some("folder-a".into()); dead.tombstone = Some(Register { value: true, revision: "r2".into() }); body.folders.insert("folder-c".into(), dead);
    body.folders.get_mut("folder-a").unwrap().position.value.parent = Some("folder-b".into());
    retained.resolved.as_mut().unwrap().parents.insert("folder-a".into(), Some("folder-b".into()));
    let observed = build_v2(&retained, &members).unwrap(); let intent = MoveIntent { node: subject("member-a"), parent: parent("folder-c"), before: None };
    let edits = staged(intent, V2Plan { folders: vec![], memberships: vec![Member::Place { member: "member-a".into(), bucket: Some("folder-c".into()), key: "a0".into() }] });
    assert!(matches!(edits.project(&observed).unwrap(), Cow::Borrowed(_)));
}
#[test]
fn membership_overlay_resolves_final_destinations_and_removals_only() {
    let (arrangement, mut members, observed) = observed_inputs();
    let place = |bucket: &str| Member::Place { member: "member-a".into(), bucket: Some(bucket.into()), key: "Zz".into() };
    let intent = MoveIntent { node: subject("member-a"), parent: parent("folder-a"), before: None };
    let edits = staged(intent.clone(), V2Plan { folders: vec![], memberships: vec![place("missing"), place("folder-a")] });
    members[0].position.bucket = Some("folder-a".into()); members[0].position.key = "Zz".into();
    assert_eq!(edits.project(&observed).unwrap().as_ref(), &build_v2(&arrangement, &members).unwrap());
    let visible = staged(intent.clone(), V2Plan { folders: vec![], memberships: vec![place("missing")] }).project(&observed).unwrap().into_owned();
    assert_eq!(visible.nodes["subject:member-a"].shown_under, Parent::Root {});
    let edits = staged(intent, V2Plan { folders: vec![], memberships: vec![place("missing"), Member::Remove { member: "member-a".into() }] });
    assert!(!edits.project(&observed).unwrap().nodes.contains_key("subject:member-a"));
}

#[test]
fn cut_cycle_withholds_folder_move_and_delete_but_not_membership_overlay() {
    let (mut arrangement, members) = inputs();
    let arrangements::VersionedBody::V2(body) = &mut arrangement.body else { unreachable!() };
    body.folders.get_mut("folder-a").unwrap().position.value.parent = Some("folder-b".into());
    body.folders.get_mut("folder-b").unwrap().position.value.parent = Some("folder-a".into());
    arrangement.resolved.as_mut().unwrap().parents.insert("folder-a".into(), Some("folder-b".into()));
    let observed = build_v2(&arrangement, &members).unwrap();
    assert_eq!(observed.nodes["folder:folder-b"].parent, parent("folder-a"));
    assert_eq!(observed.nodes["folder:folder-b"].shown_under, Parent::Root {});
    let intent = MoveIntent { node: folder("folder-a"), parent: Parent::Root {}, before: None };
    let mut edits = staged(intent.clone(), plan_v2(&observed, &intent).unwrap());
    assert!(matches!(edits.project(&observed).unwrap(), Cow::Borrowed(_)));
    let deletion = staged(intent, V2Plan { folders: vec![Folder::FolderDelete { id: "folder-b".into() }], memberships: vec![] });
    assert!(matches!(deletion.project(&observed).unwrap(), Cow::Borrowed(_)));
    let intent = MoveIntent { node: subject("member-a"), parent: parent("folder-b"), before: None };
    let membership = staged(intent.clone(), plan_v2(&observed, &intent).unwrap());
    let visible = membership.project(&observed).unwrap();
    assert!(matches!(visible, Cow::Owned(_)));
    assert_eq!(visible.nodes["subject:member-a"].shown_under, parent("folder-b"));
    // The next authoritative resolution, not a guessed resolver, reveals the moved B.
    let arrangements::VersionedBody::V2(body) = &mut arrangement.body else { unreachable!() };
    body.folders.get_mut("folder-a").unwrap().position.value.parent = None;
    arrangement.resolved.as_mut().unwrap().parents.insert("folder-a".into(), None);
    arrangement.resolved.as_mut().unwrap().parents.insert("folder-b".into(), Some("folder-a".into()));
    let confirmed = build_v2(&arrangement, &members).unwrap();
    assert_eq!(confirmed.nodes["folder:folder-a"].children, vec![folder("folder-b")]);
    edits.reduce(Event::Persisted(identity(1))); edits.reduce(Event::Observed(identity(1)));
    assert!(matches!(edits.project(&confirmed).unwrap(), Cow::Borrowed(_)));
    assert_eq!(edits.project(&confirmed).unwrap().as_ref(), &confirmed);
}

#[test]
fn creating_an_orphans_missing_parent_waits_for_authoritative_resolution() {
    let (mut arrangement, members) = inputs();
    let arrangements::VersionedBody::V2(body) = &mut arrangement.body else { unreachable!() };
    body.folders.get_mut("folder-a").unwrap().position.value.parent = Some("folder-x".into());
    let observed = build_v2(&arrangement, &members).unwrap();
    assert_eq!(observed.nodes["folder:folder-a"].parent, parent("folder-x"));
    assert_eq!(observed.nodes["folder:folder-a"].shown_under, Parent::Root {});
    let intent = MoveIntent { node: folder("folder-a"), parent: parent("folder-x"), before: None };
    let mut edits = staged(intent, V2Plan { folders: vec![Folder::FolderCreate { id: "folder-x".into(), name: "X".into(), parent: Some("folder-b".into()), key: "a0".into() }], memberships: vec![] });
    assert!(matches!(edits.project(&observed).unwrap(), Cow::Borrowed(_)));
    let arrangements::VersionedBody::V2(body) = &mut arrangement.body else { unreachable!() };
    let mut created = body.folders["folder-b"].clone();
    created.position.value = arrangements::Position { parent: Some("folder-b".into()), key: "a0".into() };
    body.folders.insert("folder-x".into(), created);
    arrangement.resolved.as_mut().unwrap().parents.insert("folder-x".into(), Some("folder-b".into()));
    arrangement.resolved.as_mut().unwrap().parents.insert("folder-a".into(), Some("folder-x".into()));
    let confirmed = build_v2(&arrangement, &members).unwrap();
    assert_eq!(confirmed.nodes["folder:folder-b"].children, vec![folder("folder-x")]);
    assert_eq!(confirmed.nodes["folder:folder-x"].children, vec![folder("folder-a")]);
    edits.reduce(Event::Persisted(identity(1))); edits.reduce(Event::Observed(identity(1)));
    assert_eq!(edits.project(&confirmed).unwrap().as_ref(), &confirmed);
}

#[test]
fn folder_create_withholds_when_a_root_members_retained_bucket_is_unavailable() {
    let (mut arrangement, mut members) = inputs();
    // The retained server Place bucket is X; live membership reads expose only root.
    let retained_bucket = Some("folder-x".to_owned());
    assert_eq!(members[0].position.bucket, None);
    let observed = build_v2(&arrangement, &members).unwrap();
    assert!(observed.nodes.values().all(|row| row.parent == row.shown_under));
    let intent = MoveIntent { node: subject("member-a"), parent: parent("folder-x"), before: None };
    let mut edits = staged(intent, V2Plan { folders: vec![Folder::FolderCreate { id: "folder-x".into(), name: "X".into(), parent: Some("folder-b".into()), key: "a0".into() }], memberships: vec![] });
    assert!(matches!(edits.project(&observed).unwrap(), Cow::Borrowed(_)));
    let arrangements::VersionedBody::V2(body) = &mut arrangement.body else { unreachable!() };
    let mut created = body.folders["folder-b"].clone();
    created.position.value = arrangements::Position { parent: Some("folder-b".into()), key: "a0".into() };
    body.folders.insert("folder-x".into(), created);
    arrangement.resolved.as_mut().unwrap().parents.insert("folder-x".into(), Some("folder-b".into()));
    members[0].position.bucket = retained_bucket;
    let confirmed = build_v2(&arrangement, &members).unwrap();
    assert_eq!(confirmed.nodes["folder:folder-x"].children, vec![subject("member-a")]);
    edits.reduce(Event::Persisted(identity(1))); edits.reduce(Event::Observed(identity(1)));
    assert_eq!(edits.project(&confirmed).unwrap().as_ref(), &confirmed);
}
