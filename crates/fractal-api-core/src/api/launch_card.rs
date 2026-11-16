//! A launch card's preview: the launch's newest variant's mission, its steps in display order
//! without finally steps, the agents they name, and how many stop for a person; or why st has no
//! mission to show (docs/clients/ui-contract.md, Attention row).

use super::*;
use crate::model::{GateSpec, MissionSpec, NormalizedIntent, PlanningSessionView, WorkSelector};

/// The most diagnostics an unavailable preview names.
const DIAGNOSTICS: usize = 3;

/// The largest preview a card carries, so a row with it stays within its 16 KiB bound: past it,
/// the card says the mission is too large to show here.
pub(crate) const LAUNCH_PREVIEW_BYTES: usize = 12 * 1024;

fn unavailable(reason: impl Into<String>, diagnostics: Vec<String>) -> Value {
    json!({"unavailable": {"reason": reason.into(), "diagnostics": diagnostics}})
}

/// A launch approval's preview, `launch` on its presentation: its newest variant's mission, or
/// `{unavailable: {reason, diagnostics}}`. `None` when st no longer has the launch. `label`
/// names an agent st knows.
pub(crate) fn launch_card(
    store: &Store,
    row: &Value,
    label: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Option<Value>> {
    let Some(launch) = row["launch_id"]
        .as_str()
        .map(|id| id.strip_prefix("launch/").unwrap_or(id))
    else {
        return Ok(None);
    };
    let Some(session) = store.planning_session(launch)? else {
        return Ok(None);
    };
    Ok(Some(newest_variant(store, &session, label)?))
}

fn newest_variant(
    store: &Store,
    session: &PlanningSessionView,
    label: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Value> {
    // The variant whose latest candidate the planner submitted last; the store lists variants
    // by name, so the newest is chosen by submission time, then by name for one instant.
    let Some(variant) = session.variants.iter().max_by(|left, right| {
        left.candidate
            .submitted_at_unix_ms
            .cmp(&right.candidate.submitted_at_unix_ms)
            .then_with(|| left.name.cmp(&right.name))
    }) else {
        return Ok(unavailable("The planner has not proposed a mission yet.", vec![]));
    };
    let Some(preview) = &variant.preview else {
        return Ok(unavailable(
            format!(
                "The planner's latest candidate ({}) has no preview yet, so there is no mission to show.",
                variant.name
            ),
            vec![],
        ));
    };
    let diagnostics = || {
        preview
            .mission
            .blockers
            .iter()
            .chain(&preview.mission.warnings)
            .take(DIAGNOSTICS)
            .cloned()
            .collect::<Vec<_>>()
    };
    // The candidate as the preview resolved it, read as the preview read it.
    let intent = match parse_intent(&preview.mission.resolved_intent.kdl, store.origin()).and_then(
        |intent| {
            Ok(match crate::store::identities::normalize_intent(&store.readers.get(), &intent)? {
                std::borrow::Cow::Borrowed(_) => intent,
                std::borrow::Cow::Owned(normalized) => normalized,
            })
        },
    ) {
        Ok(intent) => intent,
        Err(error) => {
            return Ok(unavailable(
                format!("st cannot read the proposed mission: {error}"),
                diagnostics(),
            ));
        }
    };
    let Some(mission) = intent.missions.get(&session.mission) else {
        return Ok(unavailable(
            format!(
                "The planner's latest candidate has no mission named {}.",
                session.mission
            ),
            diagnostics(),
        ));
    };
    let launch = mission_preview(store, &variant.name, &intent, mission, label)?;
    if serde_json::to_vec(&launch)?.len() > LAUNCH_PREVIEW_BYTES {
        return Ok(unavailable(
            "The proposed mission is too large to preview here: open the launch to read it.",
            vec![],
        ));
    }
    Ok(launch)
}

/// What a person reads for an agent: the name st knows, else its id without `agent/`.
fn agent_label(agent: &str, label: &impl Fn(&str) -> Option<String>) -> String {
    label(agent).unwrap_or_else(|| agent.trim_start_matches("agent/").to_owned())
}

fn mission_preview(
    store: &Store,
    variant: &str,
    intent: &NormalizedIntent,
    mission: &MissionSpec,
    label: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Value> {
    // Display order first, then any step it does not list, as the mission declares them.
    let mut order = mission
        .display_order
        .iter()
        .filter(|path| mission.steps.contains_key(*path))
        .collect::<Vec<_>>();
    order.extend(mission.steps.keys().filter(|path| !mission.display_order.contains(*path)));
    let mut named = Vec::<String>::new();
    let mut asks_you = 0;
    let steps = order
        .into_iter()
        .filter_map(|path| mission.steps.get(path))
        .filter(|step| !step.finally)
        .map(|step| {
            let assignee = match step.work_selector.as_ref().or(mission.work_selector.as_ref()) {
                Some(WorkSelector::Assigned { agent }) => {
                    named.push(agent.clone());
                    json!({"kind": "agent", "agent": agent, "label": agent_label(agent, &label)})
                }
                Some(WorkSelector::Available { agents }) => {
                    named.extend(agents.iter().cloned());
                    json!({"kind": "any", "agent": null, "label": format!("any of {} agents", agents.len())})
                }
                Some(WorkSelector::Agentless) => json!({"kind": "st", "agent": null, "label": "Fractal"}),
                None => json!({"kind": "none", "agent": null, "label": "no one"}),
            };
            let stops = step.gates.iter().any(|gate| matches!(gate, GateSpec::Human { .. }));
            asks_you += usize::from(stops);
            json!({
                "path": step.path,
                "title": step.title.as_deref().unwrap_or(&step.path),
                "assignee": assignee,
                "after": step.dependencies.iter().filter_map(|dependency| match dependency {
                    crate::model::DependencySpec::Step { step, .. } => Some(step.as_str()),
                    _ => None,
                }).collect::<Vec<_>>(),
                "asks_you": stops,
            })
        })
        .collect::<Vec<_>>();
    // Each agent once, in the order the steps name them: its seat as this candidate declares
    // it, else as st already runs it.
    let mut seen = BTreeSet::new();
    named.retain(|agent| seen.insert(agent.clone()));
    let running = store
        .desired_subjects_named(&named)?
        .into_iter()
        .filter(|subject| subject.kind == "agent")
        .map(|subject| (subject.subject.clone(), subject))
        .collect::<BTreeMap<_, _>>();
    let agents = named
        .iter()
        .map(|agent| {
            let member = intent
                .subjects
                .get(agent)
                .filter(|subject| subject.kind == "agent")
                .or_else(|| running.get(agent))
                .and_then(|subject| subject.member.as_ref());
            json!({
                "id": agent,
                "label": agent_label(agent, &label),
                "harness": member.and_then(|member| member.driver.as_deref()),
                "host": member.map(|member| member.host.as_str()).filter(|host| !host.is_empty()),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "variant": variant,
        "goals": mission.goals,
        "steps": steps,
        "agents": agents,
        "asks_you": asks_you,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_preview_lists_the_steps_in_order_and_the_agents_they_name() {
        let intent = crate::graph::parse_internal_intent(
            r#"version 2
mission "audit" state="ready" {
  goal "Audit the build."
  step "prepare" {
    assigned-to "agent/example/oak"
    goal "Prepare."
  }
  step "review" {
    assigned-to "agent/example/elm"
    depends-on { step "prepare" completed }
    goal "Review."
  }
  step "approve" {
    agentless
    depends-on { step "review" completed }
    gate "the operator approves" type="human" {
      reviewer "person/operator"
      question "Ship it?"
    }
  }
  finally {
    step "tidy" {
      agentless
    }
  }
}
"#,
            "alder",
        )
        .unwrap();
        let mission = intent.missions.values().next().unwrap();
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests_shared::state(root.path());
        let label = |agent: &str| (agent == "agent/example/oak").then(|| "Oak".to_owned());
        let launch = mission_preview(&state.store, "variant/a", &intent, mission, label).unwrap();
        assert_eq!(launch["variant"], "variant/a");
        assert_eq!(launch["goals"], json!(["Audit the build."]));
        let steps = launch["steps"].as_array().unwrap();
        // In display order, and a finally step is no step of the preview.
        assert_eq!(
            steps.iter().map(|step| step["path"].as_str().unwrap()).collect::<Vec<_>>(),
            ["prepare", "review", "approve"]
        );
        assert_eq!(
            steps[0]["assignee"],
            json!({"kind":"agent","agent":"agent/example/oak","label":"Oak"})
        );
        // An agent st does not know reads as its id without `agent/`.
        assert_eq!(steps[1]["assignee"]["label"], "example/elm");
        assert_eq!(steps[1]["after"], json!(["prepare"]));
        assert_eq!(steps[2]["assignee"], json!({"kind":"st","agent":null,"label":"Fractal"}));
        assert_eq!(
            steps.iter().map(|step| step["asks_you"].as_bool().unwrap()).collect::<Vec<_>>(),
            [false, false, true]
        );
        assert_eq!(launch["asks_you"], 1);
        assert_eq!(
            launch["agents"]
                .as_array()
                .unwrap()
                .iter()
                .map(|agent| (agent["id"].as_str().unwrap(), agent["label"].as_str().unwrap()))
                .collect::<Vec<_>>(),
            [("agent/example/oak", "Oak"), ("agent/example/elm", "example/elm")]
        );
        // Every shape the card serves decodes as the client's type.
        let decoded: fractal_client::AttentionLaunch = serde_json::from_value(launch).unwrap();
        assert!(matches!(decoded, fractal_client::AttentionLaunch::Preview(_)));
        let decoded: fractal_client::AttentionLaunch =
            serde_json::from_value(unavailable("No mission yet.", vec!["a blocker".into()])).unwrap();
        assert!(matches!(decoded, fractal_client::AttentionLaunch::Unavailable { .. }));
    }
}
