//! Complete live membership inputs exercise the same tree authority as v1.
use fractal_sidebar::model::{ArrangementInput, MembershipInput, ProjectionError};
use fractal_sidebar::projection::{build, build_v2, rollup};
use serde_json::{Value, json};

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
        assert_eq!(case["events"][0]["type"], "ProjectV2");
        assert_eq!(serde_json::to_value(&tree).unwrap(), checkpoint["tree"], "{}", case["name"]);
        let marked = serde_json::from_value(case["events"][0]["needsYou"].clone()).unwrap();
        assert_eq!(serde_json::to_value(rollup(&tree, &marked)).unwrap(), checkpoint["rollup"]);
        for field in ["moves", "effects", "wireActions"] { assert_eq!(checkpoint[field], json!([])); }
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
