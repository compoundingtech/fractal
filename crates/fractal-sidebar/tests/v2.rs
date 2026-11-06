//! Complete live membership inputs exercise the same tree authority as v1.
use fractal_sidebar::model::{ArrangementInput, MembershipInput, ProjectionError};
use fractal_sidebar::projection::{build, build_v2, rollup};
use serde_json::{Value, json};
use fractal_sidebar::plan::{MoveIntent, V2Refusal, plan_v2};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/clients/sidebar/behavior-v2.json")).unwrap()
}

fn input(case: &Value) -> (ArrangementInput, Vec<MembershipInput>) {
    let a = &case["initial"]["arrangement"];
    (ArrangementInput {
        subject: a["id"].as_str().unwrap().into(),
        owner: a["owner"].as_str().unwrap().into(),
        revision: a["revision"].as_str().unwrap().into(),
        deleted: a["deleted"].as_bool().unwrap(),
        body: serde_json::from_value(a["body"].clone()).unwrap(),
        resolved: serde_json::from_value(a["resolved"].clone()).unwrap(),
    }, serde_json::from_value(case["initial"]["memberships"].clone()).unwrap())
}

#[test]
fn nested_v2_golden_and_retirement() {
    let vectors = fixture();
    let schema: Value = serde_json::from_str(include_str!("../../../fixtures/clients/sidebar/behavior.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<_> = validator.iter_errors(&vectors).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    for case in vectors["cases"].as_array().unwrap() {
        let (arrangement, mut members) = input(case);
        let tree = build_v2(&arrangement, &members).unwrap();
        let checkpoint = &case["checkpoints"][0];
        assert!(["ProjectV2", "ProjectAndPlanV2"].contains(&case["events"][0]["type"].as_str().unwrap()));
        assert_eq!(serde_json::to_value(&tree).unwrap(), checkpoint["tree"], "{}", case["name"]);
        let marked = serde_json::from_value(case["events"][0]["needsYou"].clone()).unwrap();
        assert_eq!(serde_json::to_value(rollup(&tree, &marked)).unwrap(), checkpoint["rollup"]);
        for field in ["effects", "wireActions"] { assert_eq!(checkpoint[field], json!([])); }
        let moves = case["events"][0]["moves"].as_array().unwrap();
        let expected = checkpoint["moves"].as_array().unwrap();
        assert_eq!(moves.len(), expected.len());
        for (intent, expected) in moves.iter().zip(expected) {
            let intent = serde_json::from_value(intent.clone()).unwrap();
            assert_eq!(json!({"_tag":"PlannedV2", "edit":plan_v2(&arrangement, &members, &intent).unwrap()}), *expected);
        }
        members.reverse();
        assert_eq!(build_v2(&arrangement, &members).unwrap(), tree);
    }
    for pointer in ["/cases/0/initial/memberships/0", "/cases/0/initial/memberships/0/position", "/cases/0/initial/arrangement/body"] {
        let mut invalid = vectors.clone();
        invalid.pointer_mut(pointer).unwrap()["unknown"] = json!(true);
        assert!(!validator.is_valid(&invalid), "{pointer}");
    }
    let mut invalid = vectors;
    invalid["cases"][0]["events"][0]["type"] = json!("ProjectAndPlanV1");
    assert!(!validator.is_valid(&invalid));
}

#[test]
fn authoritative_resolution_and_scope_are_required() {
    let (arrangement, members) = input(&fixture()["cases"][0]);
    for problem in 0..6 {
        let mut a = arrangement.clone();
        let mut m = members.clone();
        match problem {
            0 => m[0].container.push_str("-other"),
            1 => m.push(m[0].clone()),
            2 => m[0].position.bucket = Some("unknown".into()),
            3 => m[0].position.bucket = Some("00000000-0000-7000-8000-00000000000d".into()),
            4 => { a.resolved.as_mut().unwrap().parents.insert("00000000-0000-7000-8000-00000000000a".into(), Some("00000000-0000-7000-8000-00000000000b".into())); }
            _ => { a.resolved.as_mut().unwrap().parents.insert("00000000-0000-7000-8000-00000000000a".into(), Some("unknown".into())); }
        }
        assert_eq!(build_v2(&a, &m), Err(ProjectionError::InvalidResolution));
    }
    let mut a = arrangement.clone();
    a.resolved = None;
    assert_eq!(build_v2(&a, &members), Err(ProjectionError::NotReady));
    a.resolved = arrangement.resolved.clone();
    a.resolved.as_mut().unwrap().parents.clear();
    assert_eq!(build_v2(&a, &members), Err(ProjectionError::NotReady));
    a.deleted = true;
    assert_eq!(build_v2(&a, &members), Err(ProjectionError::ArrangementUnavailable));
}

#[test]
fn explicit_body_versions_never_fall_back() {
    let (v2, members) = input(&fixture()["cases"][0]);
    assert_eq!(build(&v2, &["mission/remembered".into()]), Err(ProjectionError::Unsupported { required: 1, advertised: 2 }));
    let mut v1 = v2;
    v1.body = serde_json::from_value(json!({"version":1,"name":{"value":"Sidebar","revision":"r1"},"folders":{},"placements":{}})).unwrap();
    assert_eq!(build_v2(&v1, &members), Err(ProjectionError::Unsupported { required: 2, advertised: 1 }));
    for version in [0, 3, 255] {
        assert!(fractal_schema::arrangements::body_for_read(&json!({"version":version,"name":{"value":"Sidebar","revision":"r1"},"folders":{}})).is_err());
    }
}

#[test]
fn live_member_move_and_removal_reproject_without_stale_rows_or_rollups() {
    let (a, mut members) = input(&fixture()["cases"][0]);
    let outer = "00000000-0000-7000-8000-00000000000a";
    let inner = "00000000-0000-7000-8000-00000000000b";
    let marked = ["agent/test/a".into(), "agent/test/b".into()].into_iter().collect();
    for (step, expected) in [[2, 2], [2, 1], [1, 1]].into_iter().enumerate() {
        let tree = build_v2(&a, &members).unwrap();
        let counts = rollup(&tree, &marked);
        assert_eq!([counts[&format!("folder:{outer}")].needs_you, counts[&format!("folder:{inner}")].needs_you], expected);
        let row = tree.nodes.get("subject:agent/test/a");
        if step == 2 { assert!(row.is_none()); }
        else {
            let row = row.unwrap();
            assert_eq!(row.shown_under, fractal_sidebar::model::Parent::Folder { id: if step == 0 { inner } else { outer }.into() });
            assert_eq!(row.revision.as_deref(), Some(if step == 0 { "claim/test-1" } else { "claim/test-4" }));
        }
        if step == 0 {
            let member = members.iter_mut().find(|m| m.member == "agent/test/a").unwrap();
            member.position.bucket = Some(outer.into());
            member.revision = "claim/test-4".into();
        } else { members.retain(|m| m.member != "agent/test/a"); }
    }
}

fn drop_member(member: &str, folder: Option<&str>, before: Option<&str>) -> MoveIntent {
    serde_json::from_value(json!({"node":{"_tag":"Subject","subject":member},"parent":folder.map_or(json!({"_tag":"Root"}), |id| json!({"_tag":"Folder","id":id})),"before":before.map(|subject| json!({"_tag":"Subject","subject":subject}))})).unwrap()
}

#[test]
fn v2_folder_cycles_noops_and_exact_folder_operations() {
    use fractal_sidebar::plan::MoveRefusal;
    let (a, members) = input(&fixture()["cases"][0]);
    let outer = "00000000-0000-7000-8000-00000000000a";
    let inner = "00000000-0000-7000-8000-00000000000b";
    assert!(matches!(plan_v2(&a, &members, &drop_member("agent/test/a", Some(inner), Some("agent/test/b"))), Err(V2Refusal::Move { refusal: MoveRefusal::NoChange { .. } })));
    let mut intent: MoveIntent = serde_json::from_value(json!({"node":{"_tag":"Folder","id":outer},"parent":{"_tag":"Folder","id":inner},"before":null})).unwrap();
    assert!(matches!(plan_v2(&a, &members, &intent), Err(V2Refusal::Move { refusal: MoveRefusal::Cycle { .. } })));
    intent.node = fractal_sidebar::model::NodeRef::Folder { id: inner.into() };
    intent.parent = fractal_sidebar::model::Parent::Root {};
    intent.before = Some(fractal_sidebar::model::NodeRef::Folder { id: outer.into() });
    assert_eq!(serde_json::to_value(plan_v2(&a, &members, &intent).unwrap()).unwrap(), json!({"folders":[{"op":"folder.move","id":inner,"parent":null,"key":"ZU"}],"memberships":[]}));
    intent.parent = fractal_sidebar::model::Parent::Folder { id: inner.into() };
    assert!(matches!(plan_v2(&a, &members, &intent), Err(V2Refusal::Move { refusal: MoveRefusal::SelfMove { .. } })));
}

fn root_members(a: &ArrangementInput, keys: &[&str]) -> Vec<MembershipInput> {
    keys.iter().enumerate().map(|(i, key)| MembershipInput { container: a.subject.clone(), member: format!("agent/test/{i:04}"), revision: "claim/test-1".into(), position: fractal_schema::ordered_membership::Position { bucket: None, key: (*key).into() } }).collect()
}

#[test]
fn v2_minimal_rekey_can_choose_backward_and_preserves_sibling_order() {
    let (a, _) = input(&fixture()["cases"][1]);
    let members = root_members(&a, &["Zz", "a0", "a0", "a0", "b00"]);
    let edit = plan_v2(&a, &members, &drop_member(&members[4].member, None, Some(&members[2].member))).unwrap();
    assert_eq!(serde_json::to_value(edit).unwrap(), json!({"folders":[],"memberships":[
        {"op":"place","member":members[4].member,"bucket":null,"key":"Zzf"},
        {"op":"place","member":members[1].member,"bucket":null,"key":"ZzL"}
    ]}));
}

#[test]
fn v2_refuses_oversized_atomic_rekeys_and_payloads_without_truncation() {
    let (a, _) = input(&fixture()["cases"][1]);
    for (size, index, expected) in [(2046, 1023, 1024), (2050, 1025, 1026)] {
        let mut keys = vec!["a0"; size]; keys.push("b00");
        let members = root_members(&a, &keys);
        let result = plan_v2(&a, &members, &drop_member(&members[size].member, None, Some(&members[index].member)));
        if expected == 1024 { assert_eq!(result.unwrap().memberships.len(), expected); }
        else { assert_eq!(result, Err(V2Refusal::TooManyOperations { count: expected })); }
    }
    let (a, mut members) = input(&fixture()["cases"][2]);
    members[2].member = format!("agent/test/{}", "x".repeat(fractal_schema::ordered_membership::MAX_EDIT_BYTES));
    let target = members[0].position.bucket.as_deref();
    assert!(matches!(plan_v2(&a, &members, &drop_member(&members[2].member, target, Some(&members[0].member))), Err(V2Refusal::TooManyBytes { bytes }) if bytes > fractal_schema::ordered_membership::MAX_EDIT_BYTES));
}

#[test]
fn bounded_max_length_gap_rekeys_only_the_six_required_siblings() {
    let (a, _) = input(&fixture()["cases"][1]);
    let prefix = format!("a0{}", "0".repeat(fractal_schema::ordered_membership::MAX_KEY_BYTES - 3));
    let lower = format!("{prefix}1");
    let upper = format!("{prefix}z");
    let mut keys = vec![lower.as_str(); 13]; keys.extend([upper.as_str(), "b00"]);
    let members = root_members(&a, &keys);
    let edit = plan_v2(&a, &members, &drop_member(&members[14].member, None, Some(&members[7].member))).unwrap();
    assert_eq!(edit.memberships.len(), 7);
    for operation in edit.memberships {
        let fractal_schema::ordered_membership::Operation::Place { member, key, .. } = operation else { panic!("Place only") };
        assert_ne!(member, members[13].member, "the upper bound must not be rekeyed");
        assert!(key.len() <= fractal_schema::ordered_membership::MAX_KEY_BYTES && fractal_schema::arrangements::valid_key(&key));
        assert!(lower < key && key < upper);
    }
}
