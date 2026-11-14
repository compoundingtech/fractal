//! Canonical declaration text from a published structured revision, never original authored bytes.
use kdl::{KdlDocument, KdlNode};
use crate::declaration_render::{annotations, common_body, protection};
use crate::gate_render::{block, duration, node, property, text};
use crate::loop_render::{comment, dependencies, has_legacy_loop_shapes, render_loop_node};
use crate::model::{CompletionSpec, MissionInputKind, MissionSpec, MissionState, ResourceReferenceKind, RevisionCutover, FractalError, StepSpec, UsedMissionSpec};

pub struct MissionDeclarationText {
    pub text: String,
    /// Display-only legacy loop syntax, rejected by the current parser.
    pub legacy: bool,
}

pub fn render_mission_kdl(mission: &MissionSpec) -> Result<MissionDeclarationText, FractalError> {
    let mut document = KdlDocument::new();
    document.nodes_mut().extend([node("version", [2_i128.into()]), mission_node(mission)?]);
    document.autoformat();
    Ok(MissionDeclarationText { text: document.to_string(), legacy: has_legacy_loop_shapes(mission) })
}

fn mission_node(mission: &MissionSpec) -> Result<KdlNode, FractalError> {
    let mut root = text("mission", &mission.id);
    property(&mut root, "state", match mission.state { MissionState::Draft => "draft", MissionState::Ready => "ready", MissionState::Retired => "retired" });
    protection(&mut root, mission.revisions_human_only, mission.revision_reviewer.as_deref(), mission.timeout_ms);
    property(&mut root, "revision-cutover", match mission.revision_cutover { RevisionCutover::RestartActive => "restart-active", RevisionCutover::WhenIdle => "when-idle" });
    if let Some(agent) = &mission.report_to { property(&mut root, "report-to", agent.as_str()); }
    if let Some(ms) = mission.stalled_after_ms { property(&mut root, "stalled-after", format!("{ms}ms")); }
    if mission.report_to.is_some() {
        property(&mut root, "report-stalls", mission.report_stalls);
        property(&mut root, "report-completed", mission.report_completed);
    }
    annotations(&mut root, mission.declarations_kdl.as_deref(), &mission.revision_owners, &mission.agent_constraints);
    Ok(block(root, mission_body(mission, None)?))
}

fn mission_body(mission: &MissionSpec, parent: Option<&MissionSpec>) -> Result<Vec<KdlNode>, FractalError> {
    // Exhaustive destructuring makes a new MissionSpec field a renderer coverage decision.
    let MissionSpec { id: _, subject: _, state: _, revision: _, system, resources, authority, inputs,
        max_active_runs, timeout_ms: _, report_to: _, stalled_after_ms: _, report_stalls: _,
        report_completed: _, revision_owners: _, revisions_human_only: _, revision_reviewer: _,
        revision_cutover: _, declarations_kdl: _, capacity, work_selector, completion, goals,
        declared_priority, declared_lane, declared_budgets,
        constraints, agent_constraints: _, baselines, products, gates, steps, display_order } = mission;
    let goals = if parent.is_some() { goals.get(1..).ok_or_else(||
        FractalError::new("unrenderable-mission", "embedded loop mission lacks its injected goal"))? } else { goals.as_slice() };
    let mut children = common_body(goals, constraints, work_selector.as_ref(), baselines, products, gates)?;
    if let Some(priority) = declared_priority { children.push(text("declared-priority", priority.as_str())); }
    if let Some(lane) = declared_lane { children.push(text("declared-lane", lane.as_str())); }
    if let Some(budgets) = declared_budgets {
        let mut leaves = Vec::new();
        for (name, value) in [("cpu-seconds", budgets.cpu_seconds), ("token-budget", budgets.token_budget)] {
            if let Some(value) = value { leaves.push(node(name, [i128::from(value).into()])); }
        }
        for (name, value) in [("memory-max", budgets.memory_max.as_deref()), ("disk-max", budgets.disk_max.as_deref())] {
            if let Some(value) = value { leaves.push(text(name, value)); }
        }
        for entry in &budgets.attention {
            let mut attention = text("attention", &entry.person);
            property(&mut attention, "asks", i128::from(entry.asks));
            leaves.push(attention);
        }
        children.push(block(node("declared-budgets", []), leaves));
    }
    if *system { children.push(node("system", [])); }
    for reference in resources {
        let mut resource = text("resource", &reference.name);
        property(&mut resource, "subject", reference.subject.as_str());
        if let Some(reason) = &reference.reason { property(&mut resource, "reason", reason.as_str()); }
        if reference.kind == ResourceReferenceKind::Uri {
            comment(&mut resource, "authored URI edge; locator is not stored in the published revision");
        }
        children.push(resource);
    }
    if !authority.is_empty() {
        children.push(block(node("authority", []),
            authority.iter().map(|grant| text(grant.verb.as_str(), &grant.pattern)).collect()));
    }
    for input in inputs.values() {
        if parent.is_some_and(|parent| parent.inputs.contains_key(&input.name)
            || [crate::mission::LOOP_ROUND_INPUT, crate::mission::LOOP_FEEDBACK_INPUT,
                crate::mission::LOOP_ITEM_INPUT, crate::mission::CANDIDATE_INDEX_INPUT].contains(&input.name.as_str())) { continue; }
        let mut input_node = text("input", &input.name);
        property(&mut input_node, "kind", match input.kind { MissionInputKind::Text => "text", MissionInputKind::Resource => "resource" });
        children.push(input_node);
    }
    if parent.is_none() {
        let mut concurrent = node("concurrent-runs", []);
        if let Some(max) = max_active_runs { property(&mut concurrent, "max", i128::from(*max)); }
        children.push(concurrent);
    }
    if let Some(capacity) = capacity {
        let mut capacity_node = node("capacity", []);
        property(&mut capacity_node, "cores", i128::from(capacity.cores));
        property(&mut capacity_node, "memory", fractal_runtime::show_lane_memory(capacity.memory));
        children.push(capacity_node);
    }
    if let Some(completion) = completion {
        let body = match completion {
            CompletionSpec::AllStepsExhausted => vec![text("when", "all-steps-exhausted")],
            CompletionSpec::Dependencies { dependencies: after } => vec![block(node("depends-on", []), dependencies(after)?)],
        };
        children.push(block(node("completion", []), body));
    }
    if display_order.len() != steps.len() || display_order.iter().collect::<std::collections::BTreeSet<_>>().len() != steps.len() {
        return Err(FractalError::new("unrenderable-mission", "display order does not enumerate every step exactly once"));
    }
    for id in display_order {
        let step = steps.get(id).ok_or_else(|| FractalError::new("unrenderable-mission", "display order names an absent step"))?;
        let rendered = if let Some(spec) = &step.loop_spec {
            render_loop_node(spec, &step.dependencies, &|branch| mission_body(branch, Some(mission)))?
        } else { step_node(step)? };
        let group = if step.finally { Some(("finally", None)) } else { step.queue.as_deref().map(|queue| ("queue", Some(queue))) };
        if let Some((name, argument)) = group {
            if let Some(last) = children.last_mut().filter(|last| last.name().value() == name
                && last.get(0).and_then(|value| value.as_string()) == argument
                // Each authored queue block resets positions, even when its name repeats.
                && (name != "queue" || step.queue_position != Some(1))) {
                last.ensure_children().nodes_mut().push(rendered);
            } else {
                let root = match argument { Some(argument) => text(name, argument), None => node(name, []) };
                children.push(block(root, vec![rendered]));
            }
        } else { children.push(rendered); }
    }
    if parent.is_some() && let Some(first) = children.first_mut() {
        annotations(first, mission.declarations_kdl.as_deref(), &mission.revision_owners, &mission.agent_constraints);
    }
    Ok(children)
}

fn step_node(step: &StepSpec) -> Result<KdlNode, FractalError> {
    let StepSpec { id, path: _, fresh_context, keep_open, queue: _, queue_position: _, title,
        goals, constraints, agent_constraints, timeout_ms, capacity, retry, finally: _,
        work_selector, revision_owners, revisions_human_only, revision_reviewer, dependencies: after,
        baselines, documents, declarations_kdl, products, produces_mission, uses_mission, after_run,
        program, gates, nested_mission, loop_spec: _, definition_hash: _ } = step;
    let mut root = text("step", id);
    protection(&mut root, *revisions_human_only, revision_reviewer.as_deref(), *timeout_ms);
    annotations(&mut root, declarations_kdl.as_deref(), revision_owners, agent_constraints);
    let mut children = common_body(goals, constraints, work_selector.as_ref(), baselines, products, gates)?;
    for (name, value) in [("fresh-context", fresh_context), ("keep-open", keep_open)] {
        if *value { children.push(node(name, [])); }
    }
    if let Some(title) = title { children.push(text("title", title)); }
    if let Some(capacity) = capacity {
        let mut capacity_node = node("capacity", []);
        if let Some(cores) = capacity.cores { property(&mut capacity_node, "cores", i128::from(cores)); }
        if let Some(memory) = capacity.memory { property(&mut capacity_node, "memory", fractal_runtime::show_lane_memory(memory)); }
        children.push(capacity_node);
    }
    if !after.is_empty() { children.push(block(node("depends-on", []), dependencies(after)?)); }
    children.extend(documents.iter().map(|document| text("document", document)));
    children.push(block(node("retry", []), vec![node("attempts", [i128::from(retry.attempts).into()]), duration("backoff", retry.backoff_ms)]));
    if let Some(produced) = produces_mission { children.push(text("produces-mission", produced)); }
    if let Some(used) = uses_mission {
        children.push(match used {
            UsedMissionSpec::Revision { mission, revision } => text("uses-mission", &format!("{mission}@{revision}")),
            UsedMissionSpec::StepOutput { step } => { let mut node = node("uses-mission", []); property(&mut node, "output-of", step.as_str()); node }
        });
    }
    if let Some(run) = after_run { children.push(text("after-run", run)); }
    if let Some(program) = program {
        let mut program_node = text("program", &program.bundle);
        property(&mut program_node, "sha256", program.bundle_hash.as_str());
        if let Some(ms) = program.stalled_after_ms { property(&mut program_node, "stalled-after", format!("{ms}ms")); }
        children.push(program_node);
    }
    if let Some(nested) = nested_mission { children.push(mission_node(nested)?); }
    Ok(block(root, children))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parse(source: &str) -> MissionSpec {
        crate::mission::parse_missions(&source.parse().unwrap(), "node").unwrap().remove("test").unwrap()
    }

    fn visible(mut value: Value) -> Value {
        match &mut value {
            Value::Object(fields) => {
                if fields.contains_key("display_order") || fields.contains_key("definition_hash") {
                    for name in ["revision", "definition_hash", "declarations_kdl", "revision_owners", "agent_constraints"] { fields.remove(name); }
                }
                if fields.get("kind").and_then(Value::as_str) == Some("uri") && fields.contains_key("name") {
                    fields.insert("kind".into(), Value::String("subject".into()));
                }
                if let Some(Value::Object(env)) = fields.get_mut("environment") {
                    for value in env.values_mut() { *value = Value::String("<redacted>".into()); }
                }
                for value in fields.values_mut() { *value = visible(value.take()); }
            }
            Value::Array(values) => for value in values { *value = visible(value.take()); },
            _ => {}
        }
        value
    }

    fn round_trip(source: &str) {
        let spec = parse(source);
        let rendered = render_mission_kdl(&spec).unwrap();
        assert!(!rendered.legacy);
        assert!(!rendered.text.contains("secret"));
        let reread = parse(&rendered.text);
        assert_eq!(visible(serde_json::to_value(&spec).unwrap()), visible(serde_json::to_value(&reread).unwrap()));
        assert_eq!(render_mission_kdl(&spec).unwrap().text, rendered.text);
    }

    #[test]
    fn canonical_declared_admission_round_trip() {
        round_trip(r#"mission "test" state="ready" {
            goal "Keep declaration metadata."
            declared-priority "normal"
            declared-lane "check"
            declared-budgets { cpu-seconds 300; token-budget 100000; memory-max "8G"; disk-max "20G"; attention "person/reviewer" asks=3; }
        }"#);
    }

    #[test]
    fn canonical_mission_authority_round_trip() {
        let source = r#"mission "test" state="ready" {
            goal "Land one change."
            authority { write "resource/factory/*"; publish "scratch/*"; start "factory/publish"; }
            step "work" { agentless; }
        }"#;
        round_trip(source);
        let rendered: KdlDocument = render_mission_kdl(&parse(source)).unwrap().text.parse().unwrap();
        let rules = rendered.get("mission").unwrap().children().unwrap().get("authority").unwrap()
            .children().unwrap().nodes().iter()
            .map(|rule| format!("{} {}", rule.name().value(), rule.get(0).unwrap().as_string().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(rules, ["publish scratch/*", "start factory/publish", "write resource/factory/*"]);
    }

    #[test]
    fn canonical_declared_admission_retains_typed_metadata() {
        let mut spec = parse(r#"mission "test" state="ready" { goal "Keep declaration metadata."; }"#);
        spec.declared_priority = Some(crate::model::DeclaredPriority::Unknown("future-priority".into()));
        spec.declared_lane = Some(crate::model::DeclaredLane::Build);
        spec.declared_budgets = Some(crate::model::DeclaredBudgets {
            cpu_seconds: Some(300),
            token_budget: Some(100_000),
            memory_max: Some("8G".into()),
            disk_max: Some("20G".into()),
            ..Default::default()
        });
        let rendered = render_mission_kdl(&spec).unwrap();
        let document: KdlDocument = rendered.text.parse().unwrap();
        let body = document.get("mission").unwrap().children().unwrap();
        assert_eq!(body.get("declared-priority").unwrap().get(0).unwrap().as_string(), Some("future-priority"));
        assert_eq!(body.get("declared-lane").unwrap().get(0).unwrap().as_string(), Some("build"));
        let budgets = body.get("declared-budgets").unwrap().children().unwrap();
        assert_eq!(budgets.get("cpu-seconds").unwrap().get(0).unwrap().as_integer(), Some(300));
        assert_eq!(budgets.get("token-budget").unwrap().get(0).unwrap().as_integer(), Some(100_000));
        assert_eq!(budgets.get("memory-max").unwrap().get(0).unwrap().as_string(), Some("8G"));
        assert_eq!(budgets.get("disk-max").unwrap().get(0).unwrap().as_string(), Some("20G"));
    }

    #[test]
    fn canonical_adjacent_same_named_queue_blocks_round_trip() {
        round_trip(r#"mission "test" state="ready" {
          goal "Keep the independent queues separate."
          queue "q" { agentless; step "a"; step "a-next"; }
          queue "q" { agentless; step "b"; step "b-next"; }
        }"#);
    }

    #[test]
    fn canonical_mission_steps_resources_nested_queues_and_finally() {
        round_trip(r#"mission "test" state="draft" timeout="30s" revisions="human-only" revision-reviewer="person/test" revision-cutover="when-idle" report-to="agent/test" stalled-after="5s" report-stalls=#false report-completed=#true {
          goal "One \"quoted\" goal\nwith a newline."; constraint "Keep the contract."; system
          input "text" kind="text"; input "resource" kind="resource"; concurrent-runs max=2
          capacity cores=2 memory="1G"
          resource "explicit" subject="resource/test" reason="A reference."
          resource "uri" uri="https://example.com/test" reason="A URI."
          assigned-to "agent/test"
          agent "helper" { constraint "A helper constraint."; workspace "."; command "true"; env { TOKEN "secret"; }; }
          baseline "before" { exists "file/node:/test"; field "state" "file/node:/test" "is" "ready"; }
          produces { message "message/test" { text "done"; }; }
          gate "bounded" { deadline "20s"; }
          completion { depends-on "work"; }
          step "work" timeout="10s" revisions="human-only" revision-reviewer="person/test" {
            title "Work"; goal "Do the work."; constraint "Do not drop fields."; fresh-context; keep-open
            capacity cores=3 memory="2G"; available-to "agent/a"; available-to "agent/b"
            depends-on { exists "file/node:/test"; }; baseline "start" { exists "file/node:/test"; }
            document "doc/test@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; retry { attempts 3; backoff "2ms"; }
            agent "worker" { constraint "Keep this visible."; workspace "."; command "secret embedded declaration"; }
            produces { message "message/step" { text "complete"; }; }
            produces-mission "produced"
            mission "nested" state="ready" { goal "Nested."; step "inner" { agentless; }; }
            gate "run" { exec "true"; host "node"; workspace "."; env { TOKEN "secret"; }; }
          }
          step "use" { depends-on "work"; uses-mission output-of="work"; }
          step "wait" { after-run "mission-run/test/prior"; }
          queue "ordered" { agentless; step "first"; step "second"; }
          finally { step "cleanup" { agentless; }; }
        }"#);
    }

    #[test]
    fn canonical_loop_closure_inheritance_and_redaction() {
        round_trip(r#"mission "test" state="ready" timeout="30s" {
          goal "Loop."; input "parent" kind="text"
          loop "retry" timeout="10s" { max-rounds 3; depends-on { exists "file/node:/test"; }
            metric "score" direction="higher" { field "score" "resource/test"; }
            metric "exec" direction="lower" { exec "echo 1" { host "node"; workspace "."; env { TOKEN "secret"; }; }; }
            until { gate "ready" { exists "file/node:/test"; }; }
            round { input "local" kind="text"; completion { when "all-steps-exhausted"; }
              agent "helper" { workspace "."; command "secret"; }
              step "nested" { mission "nested" state="ready" { goal "Nested."; step "done" { agentless; }; }; }
            }
            keep-if metric="score"
            on-keep { completion { when "all-steps-exhausted"; }; step "keep" { agentless; }; }
            on-discard { completion { when "all-steps-exhausted"; }; step "discard" { agentless; }; }
            on-exhausted { gate "stop" type="human" { reviewer "person/test"; }; }
          }
        }"#);
    }

    #[test]
    fn canonical_unbounded_runs_and_pinned_mission_use() {
        let source = format!(r#"mission "test" state="retired" {{ goal "test"; concurrent-runs;
            available-to "agent/test"; step "use" {{ uses-mission "other@{}"; }} }}"#, "a".repeat(64));
        round_trip(&source);
        let mut spec = parse(&source);
        spec.display_order.clear();
        assert!(render_mission_kdl(&spec).is_err());
    }

    #[test]
    fn program_step_round_trips_and_owns_its_outcome() {
        let sha = "ab".repeat(32);
        let source = format!(r#"mission "test" state="ready" {{ goal "Land."; input "change" kind="text"
            step "program" timeout="1h" {{ program "/bundles/land.mjs" sha256="{sha}" stalled-after="10m"; }} }}"#);
        round_trip(&source);
        let step = &parse(&source).steps["program"];
        let program = step.program.as_ref().unwrap();
        assert_eq!((program.bundle_hash.as_str(), program.stalled_after_ms), (sha.as_str(), Some(600_000)));
        assert_eq!(step.work_selector, Some(crate::model::WorkSelector::Agentless));
        // The definition names the blob by `bundle_hash`, so publishing it promotes the blob.
        assert_eq!(serde_json::to_value(step).unwrap()["program"]["bundle_hash"], sha.as_str());
        round_trip(&format!(r#"mission "test" state="ready" {{ goal "Land."; step "program" {{ program "/b.mjs" sha256="{sha}"; }} }}"#));
        let error = |body: &str| {
            let source = format!(r#"mission "test" state="ready" {{ goal "Land."; step "program" {{ {body} }} }}"#);
            crate::mission::parse_missions(&source.parse().unwrap(), "node").unwrap_err().code
        };
        assert_eq!(error(&format!(r#"program "/b.mjs" sha256="{sha}"; assigned-to "agent/test";"#)), "conflicting-work-selector");
        assert_eq!(error(&format!(r#"program "/b.mjs" sha256="{sha}"; gate "g" {{ exists "resource/x"; }};"#)), "program-step-outcome");
        assert_eq!(error(&format!(r#"program "/b.mjs" sha256="{sha}"; uses-mission "other@{sha}";"#)), "program-step-outcome");
        assert_eq!(error(&format!(r#"program "/b.mjs" sha256="{sha}"; after-run "mission-run/x";"#)), "program-step-outcome");
        assert_eq!(error(&format!(r#"program "/b.mjs" sha256="{sha}"; mission "n" state="ready" {{ goal "N."; step "s" {{ agentless; }}; }};"#)), "program-step-outcome");
        assert_eq!(error(r#"program "/b.mjs" sha256="ABC";"#), "invalid-program");
        assert_eq!(error(&format!(r#"program "b.mjs" sha256="{sha}";"#)), "invalid-program");
        assert_eq!(error(&format!(r#"program "/b.mjs" sha256="{sha}" stalled-after="0s";"#)), "invalid-program");
        let in_finally = format!(r#"mission "test" state="ready" {{ goal "Land."; step "work" {{ agentless; }}
            finally {{ step "program" {{ program "/b.mjs" sha256="{sha}"; }} }} }}"#);
        assert_eq!(crate::mission::parse_missions(&in_finally.parse().unwrap(), "node").unwrap_err().code, "program-step-outcome");
    }

    #[test]
    fn canonical_legacy_flag_and_unicode_annotation_are_safe() {
        let mut mission = parse(r#"mission "test" state="ready" { goal "test"; loop "old" { max-rounds 1; round { completion { when "all-steps-exhausted"; }; }; }; }"#);
        mission.agent_constraints.insert("agent/test".into(), vec!["keep\u{2028}step \"injected\"".into()]);
        mission.steps.get_mut("old").unwrap().loop_spec.as_mut().unwrap().for_each =
            Some(crate::model::LoopForEachSpec { resource: "resource/items".into(), field: "values".into(), max_parallel: 2 });
        let rendered = render_mission_kdl(&mission).unwrap();
        assert!(rendered.legacy);
        assert!(rendered.text.contains("// legacy loop shape; the current parser rejects it"));
        assert!(!rendered.text.contains('\u{2028}'));
        let document: KdlDocument = rendered.text.parse().unwrap();
        assert_eq!(document.nodes().len(), 2);
        assert_eq!(crate::mission::parse_missions(&document, "node").unwrap_err().code, "removed-loop-shape");
    }
}
