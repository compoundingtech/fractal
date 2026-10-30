#![allow(unused_imports)]
use crate::store::*;
use crate::resource::github_workflows::PerformanceFailure;
use crate::store::github_workflow_failures::*;
use crate::graph::parse_test_intent as parse_intent;
use crate::resource::github_workflows::PERFORMANCE_FAILURES_FIELD;

pub const SOURCE: &str = r#"version 2
resource "repo" { kind "vcs.repository" }
observer "repo" { resource "resource/repo"; provider "github.repository"; locator "acme/garden"; field "issues" }
agent "target" { workspace "."; command "true" }
subscription "performance" { observer "observer/repo"; on "main_performance_failures"; to "agent/node.target"; delivery "message" }
"#;

pub fn publish(store: &Store, source: &str, key: &str) {
        let intent = parse_intent(source, "node").unwrap();
        let plan = store
            .mission(
                &intent,
                IntentInput {
                    kdl: source.into(),
                    source_name: None,
                },
            )
            .unwrap();
        assert!(plan.blockers.is_empty());
        let recipient = intent
            .subjects
            .values()
            .filter_map(|desired| crate::graph::subscription_spec(&desired.desired))
            .find(|spec| !spec.stopped)
            .map(|spec| spec.to)
            .unwrap_or_else(|| "agent/node.target".into());
        store
            .apply_as(&intent, &plan.subject_tokens, key, Some(&recipient))
            .unwrap();
    }

pub fn failure(run_id: u64, run_attempt: u64, head: &str) -> Value {
        json!({"repository": "acme/garden", "run_id": run_id, "run_attempt": run_attempt,
            "head_sha": head.repeat(40), "workflow": "Performance", "workflow_path": ".github/workflows/perf.yml",
            "event": "push", "head_branch": "main", "status": "completed", "conclusion": "failure",
            "url": format!("https://github.com/acme/garden/actions/runs/{run_id}")})
    }
