use super::*;
use sha2::{Digest, Sha256};

// Pinned pre-admission MissionSpec wire representation, including declaration field order.
const OLD_MISSION: &str = r#"{"id":"old","subject":"mission/old","state":"ready","revision":"","inputs":{},"max_active_runs":1,"revision_owners":[],"revisions_human_only":false,"revision_cutover":"restart-active","goals":["Preserve old bytes."],"constraints":[],"baselines":[],"products":[],"gates":[],"steps":{},"display_order":[]}"#;

#[test]
fn declared_admission_old_mission_keeps_pinned_bytes_and_hash() {
    let mut mission: MissionSpec = serde_json::from_str(OLD_MISSION).unwrap();
    assert!(mission.declared_priority.is_none());
    assert!(mission.declared_lane.is_none());
    assert!(mission.declared_budgets.is_none());
    assert_eq!(serde_json::to_string(&mission).unwrap(), OLD_MISSION);
    assert_eq!(
        hex::encode(Sha256::digest(serde_json::to_vec(&mission).unwrap())),
        "b4c167a923deec7cd788326dd59e8c5dedae0fdf9f3be29e0c5875a87f5cd1f5"
    );
    mission.declared_budgets = Some(DeclaredBudgets::default());
    assert_eq!(serde_json::to_string(&mission).unwrap(), OLD_MISSION);
}

#[test]
fn declared_admission_open_enums_keep_unknown_strings() {
    let priority: DeclaredPriority = serde_json::from_str("\"future-priority\"").unwrap();
    assert_eq!(
        priority,
        DeclaredPriority::Unknown("future-priority".into())
    );
    assert_eq!(
        serde_json::to_string(&priority).unwrap(),
        "\"future-priority\""
    );
    let lane: DeclaredLane = serde_json::from_str("\"future-lane\"").unwrap();
    assert_eq!(lane, DeclaredLane::Unknown("future-lane".into()));
    assert_eq!(serde_json::to_string(&lane).unwrap(), "\"future-lane\"");
    for (raw, expected) in [
        ("background", DeclaredPriority::Background),
        ("normal", DeclaredPriority::Normal),
    ] {
        assert_eq!(
            serde_json::from_value::<DeclaredPriority>(Value::String(raw.into())).unwrap(),
            expected
        );
    }
}

#[test]
fn declared_admission_projection_omits_empty_members_and_round_trips() {
    let empty = DeclaredBudgets::default();
    assert!(DeclaredAdmission::from_declaration(None, None, None).is_none());
    assert!(DeclaredAdmission::from_declaration(None, None, Some(&empty)).is_none());
    let group =
        DeclaredAdmission::from_declaration(Some(&DeclaredPriority::Normal), None, Some(&empty))
            .unwrap();
    assert_eq!(
        serde_json::to_value(group).unwrap(),
        serde_json::json!({"priority":"normal"})
    );
    let mut mission: MissionSpec = serde_json::from_str(OLD_MISSION).unwrap();
    let absent = serde_json::to_value(MissionDefinitionView {
        mission: mission.clone(),
        updated_at_unix_ms: 1,
    })
    .unwrap();
    assert!(absent["mission"].get("declared_admission").is_none());
    mission.declared_priority = Some(DeclaredPriority::Normal);
    mission.declared_lane = Some(DeclaredLane::Check);
    mission.declared_budgets = Some(DeclaredBudgets {
        memory_max: Some("008G".into()),
        cpu_seconds: Some(300),
        ..Default::default()
    });
    let view = MissionDefinitionView {
        mission: mission.clone(),
        updated_at_unix_ms: 1,
    };
    let json = serde_json::to_value(view).unwrap();
    assert_eq!(
        json["mission"]["declared_admission"],
        serde_json::json!({"priority":"normal","lane":"check","budgets":{"cpu_seconds":300,"memory_max":"008G"}})
    );
    for key in ["declared_priority", "declared_lane", "declared_budgets"] {
        assert!(json["mission"].get(key).is_none());
    }
    let decoded: MissionDefinitionView = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.mission, mission);
}

#[test]
fn declared_admission_typed_nulls_and_unknown_budget_keys_are_refused() {
    for field in ["declared_priority", "declared_lane", "declared_budgets"] {
        let mut value: Value = serde_json::from_str(OLD_MISSION).unwrap();
        value[field] = Value::Null;
        assert!(
            serde_json::from_value::<MissionSpec>(value).is_err(),
            "{field}"
        );
    }
    for field in ["cpu_seconds", "memory_max", "disk_max", "token_budget"] {
        assert!(
            serde_json::from_value::<DeclaredBudgets>(serde_json::json!({field: null})).is_err(),
            "{field}"
        );
    }
    assert!(serde_json::from_value::<DeclaredBudgets>(serde_json::json!({"unknown": 1})).is_err());
}

#[test]
fn declared_admission_old_agent_keeps_pinned_bytes_and_hash() {
    const OLD_MEMBER: &str = r#"{"kind":"agent","host":"node","runtime_id":"old","workspace":"/work/old","workspace_create":false,"cwd":"/work/old","terminal":false,"launch":{"type":"shell","value":"true"},"environment":{},"tags":{},"display_name":null,"lifecycle":"service","restart":"never","restart_intensity":{"attempts":3,"interval_ms":60000,"delay_ms":0,"mode":"delay"},"shutdown_timeout_ms":5000,"driver":null}"#;
    let mut member: MemberSpec = serde_json::from_str(OLD_MEMBER).unwrap();
    assert!(member.declared_priority.is_none());
    assert!(member.declared_lane.is_none());
    assert!(member.declared_budgets.is_none());
    assert_eq!(serde_json::to_string(&member).unwrap(), OLD_MEMBER);
    assert_eq!(
        hex::encode(Sha256::digest(serde_json::to_vec(&member).unwrap())),
        "16d74ecb308e94290b387ed215bfefcdc5df46b35ec7f4349c0d6e9a170bd6c4"
    );
    member.declared_budgets = Some(DeclaredBudgets::default());
    assert_eq!(serde_json::to_string(&member).unwrap(), OLD_MEMBER);
}
