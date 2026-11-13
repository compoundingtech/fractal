use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Facts read from a whole attention row, as st reads them from the row it serves.
fn row(fields: Value) -> Facts {
    let mut row = json!({
        "id": "attention/0123", "kind": "attention", "revision": "e1",
        "updated_at": "2026-10-15T09:00:00Z", "source_id": "step-run/example/audit/1/review",
        "person_id": "person/avery", "title": "Review the audit", "detail": "", "priority": "normal",
        "state": "open", "requested_at": "2026-10-15T09:00:00Z", "actions": [], "alert": true,
    });
    row.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    Facts::deserialize(&row).unwrap()
}

#[derive(Default)]
struct Known {
    labels: BTreeMap<&'static str, &'static str>,
    claimants: BTreeMap<&'static str, &'static str>,
    working: BTreeMap<&'static str, &'static str>,
    paths: BTreeMap<&'static str, &'static str>,
}

impl Fleet for Known {
    fn agent_label(&self, agent: &str) -> Option<String> {
        self.labels.get(agent).map(|label| (*label).to_owned())
    }
    fn claimant(&self, step: &str) -> Option<String> {
        self.claimants.get(step).map(|agent| (*agent).to_owned())
    }
    fn working_in(&self, mission: &str) -> Option<String> {
        self.working.get(mission).map(|agent| (*agent).to_owned())
    }
    fn step_path(&self, step: &str) -> Option<String> {
        self.paths.get(step).map(|path| (*path).to_owned())
    }
}

#[test]
fn each_kind_has_its_group_and_card() {
    for (fields, tier, kind) in [
        (
            json!({"attention_kind": "human-gate"}),
            Some(Tier::Stopped),
            Some(Kind::Review),
        ),
        (
            json!({"attention_kind": "human-gate", "review_mode": "feedback"}),
            Some(Tier::Stopped),
            Some(Kind::Feedback),
        ),
        (
            json!({"attention_kind": "human-gate", "review_mode": "approve"}),
            Some(Tier::Stopped),
            Some(Kind::Review),
        ),
        (
            json!({"attention_kind": "person-step"}),
            Some(Tier::Stopped),
            Some(Kind::Request),
        ),
        (
            json!({"attention_kind": "person-step", "update": {"type": "update"}}),
            Some(Tier::Later),
            Some(Kind::Update),
        ),
        (
            json!({"attention_kind": "agent-request"}),
            Some(Tier::Stopped),
            Some(Kind::Request),
        ),
        (
            json!({"attention_kind": "launch-approval"}),
            Some(Tier::Today),
            Some(Kind::Launch),
        ),
        (
            json!({"attention_kind": "revision-approval"}),
            Some(Tier::Today),
            Some(Kind::Revision),
        ),
        (
            json!({"attention_kind": "custom.deploy-window"}),
            Some(Tier::Today),
            Some(Kind::Request),
        ),
        (
            json!({"attention_kind": "harness-prompt"}),
            Some(Tier::Stopped),
            Some(Kind::Prompt),
        ),
        (
            json!({"attention_kind": "harness-login"}),
            Some(Tier::Stopped),
            Some(Kind::Login),
        ),
        (json!({"attention_kind": "fault"}), None, Some(Kind::Fault)),
        (json!({"attention_kind": "unread-message"}), None, None),
    ] {
        let presented = present(&row(fields.clone()), &Known::default());
        assert_eq!((presented.tier, presented.kind), (tier, kind), "{fields}");
    }
}

#[test]
fn thread_responsibility_uses_only_the_daemon_assignment() {
    let known = Known {
        labels: BTreeMap::from([("agent/example/auditor", "Auditor")]),
        claimants: BTreeMap::from([("step-run/example/audit/1/review", "agent/example/auditor")]),
        working: BTreeMap::from([("mission/example/audit", "agent/example/builder")]),
        ..Known::default()
    };
    for kind in ["human-gate", "person-step", "fault", "harness-login", "harness-prompt"] {
        let mut facts = json!({"attention_kind": kind, "mission_id": "mission/example/audit",
            "step_run_id": "step-run/example/audit/1/review", "requester_id": "agent/example/planner"});
        assert_eq!(present(&row(facts.clone()), &known).agent, None,
            "{kind}: source context cannot replace missing discussion responsibility");
        let mut source_only = facts.clone();
        source_only.as_object_mut().unwrap().remove("requester_id");
        let presented = present(&row(source_only), &known);
        assert_eq!(presented.agent, None);
        assert_eq!(presented.raised_by, Some(Who {
            id: "agent/example/auditor".into(), label: "Auditor".into(),
        }), "source attribution remains independent of discussion responsibility");
        facts["conversation_id"] = json!("agent/example/auditor");
        assert_eq!(present(&row(facts.clone()), &known).agent, Some(Who {
            id: "agent/example/auditor".into(), label: "Auditor".into(),
        }));
        facts["conversation_id"] = json!("agent/example/historical");
        assert_eq!(present(&row(facts), &known).agent, Some(Who {
            id: "agent/example/historical".into(), label: "example/historical".into(),
        }), "a retained assignment remains visible even outside the current roster");
    }
}

#[test]
fn raised_by_names_the_requester_then_the_agent_then_the_mission() {
    let known = Known {
        labels: BTreeMap::from([("agent/example/planner", "Planner")]),
        ..Known::default()
    };
    let raised = |fields: Value| present(&row(fields), &known).raised_by;
    assert_eq!(
        raised(
            json!({"attention_kind": "launch-approval", "requester_id": "agent/example/planner",
            "conversation_id": "agent/example/other"})
        ),
        Some(Who {
            id: "agent/example/planner".into(),
            label: "Planner".into()
        })
    );
    assert_eq!(
        raised(json!({"attention_kind": "person-step", "requester_id": "person/blair"})),
        Some(Who {
            id: "person/blair".into(),
            label: "person/blair".into()
        })
    );
    assert_eq!(
        raised(
            json!({"attention_kind": "harness-prompt", "conversation_id": "agent/example/planner"})
        ),
        Some(Who {
            id: "agent/example/planner".into(),
            label: "Planner".into()
        })
    );
    for actor in ["daemon/runtime", "agent/st3/reconciler"] {
        assert_eq!(
            raised(json!({"attention_kind": "fault", "requester_id": actor})),
            Some(Who {
                id: actor.into(),
                label: "st".into()
            })
        );
    }
    assert_eq!(
        raised(json!({"attention_kind": "human-gate", "mission_id": "mission/example/audit"})),
        Some(Who {
            id: "mission/example/audit".into(),
            label: "the example/audit mission".into()
        })
    );
    assert_eq!(raised(json!({"attention_kind": "harness-login"})), None);
}

#[test]
fn waiting_names_the_step_it_holds() {
    let known = Known {
        paths: BTreeMap::from([("step-run/example/audit/1/review", "review")]),
        ..Known::default()
    };
    let waiting = |fields: Value| present(&row(fields), &known).waiting;
    assert_eq!(
        waiting(
            json!({"attention_kind": "human-gate", "step_run_id": "step-run/example/audit/1/review"})
        ),
        Some(Waiting {
            step: "step-run/example/audit/1/review".into(),
            path: "review".into()
        })
    );
    // A step st no longer has holds nothing a person can read.
    assert_eq!(
        waiting(json!({"attention_kind": "human-gate", "step_run_id": "step-run/example/gone"})),
        None
    );
    assert_eq!(waiting(json!({"attention_kind": "harness-prompt"})), None);
}

#[test]
fn a_presentation_serializes_as_the_contract_names_it() {
    let presented = Presentation {
        tier: Some(Tier::Stopped),
        kind: Some(Kind::Review),
        agent: Some(Who {
            id: "agent/example/auditor".into(),
            label: "Auditor".into(),
        }),
        raised_by: None,
        waiting: Some(Waiting {
            step: "step-run/example/audit/1/review".into(),
            path: "review".into(),
        }),
    };
    assert_eq!(
        serde_json::to_value(&presented).unwrap(),
        json!({"tier": "stopped", "kind": "review",
            "agent": {"id": "agent/example/auditor", "label": "Auditor"}, "raised_by": null,
            "waiting": {"step": "step-run/example/audit/1/review", "path": "review"}})
    );
}
