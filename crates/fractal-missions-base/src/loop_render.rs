//! Render loop policy, including the last accepted syntax of published legacy loop shapes.
use kdl::KdlNode;
use crate::gate_render::{block, duration, node, property, render_gate_node, text};
use crate::model::{DependencySpec, LoopCandidateSelector, LoopExhaustionSpec, LoopSpec, MetricSource, MissionSpec, St3Error};

pub(crate) fn comment(node: &mut KdlNode, message: &str) {
    let mut format = node.format().cloned().unwrap_or_default();
    format.leading.push_str(&format!("// {message}\n"));
    node.set_format(format);
}

pub(crate) fn dependencies(values: &[DependencySpec]) -> Result<Vec<KdlNode>, St3Error> {
    values.iter().map(|dependency| Ok(match dependency {
        DependencySpec::Step { step, state } => node("step", [step.as_str().into(), state.as_str().into()]),
        DependencySpec::Predicate { gate } => render_gate_node(gate)?.children().unwrap().nodes()[0].clone(),
    })).collect()
}

/// `mission` renders an embedded mission's body, not its synthetic identity or inherited limits.
/// The caller owns recursion so nested missions and loops share exactly one mission renderer.
pub fn render_loop_node(
    spec: &LoopSpec,
    after: &[DependencySpec],
    mission: &impl Fn(&MissionSpec) -> Result<Vec<KdlNode>, St3Error>,
) -> Result<KdlNode, St3Error> {
    let LoopSpec { id, path: _, max_rounds, timeout_ms, for_each, candidates, metrics, stop,
        until, round, keep_metric, on_keep, on_discard, on_exhausted, exhaustion_attention } = spec;
    let mut root = text("loop", id);
    if let Some(ms) = timeout_ms { property(&mut root, "timeout", format!("{ms}ms")); }
    if for_each.is_some() || candidates.is_some() {
        comment(&mut root, "legacy loop shape; the current parser rejects it");
    }
    if let Some(each) = for_each {
        property(&mut root, "for-each", each.resource.as_str());
        property(&mut root, "field", each.field.as_str());
        property(&mut root, "max-parallel", i128::from(each.max_parallel));
    }
    let mut children = vec![node("max-rounds", [i128::from(*max_rounds).into()])];
    if !after.is_empty() { children.push(block(node("depends-on", []), dependencies(after)?)); }
    for metric in metrics {
        let mut root = text("metric", &metric.name);
        property(&mut root, "direction", metric.direction.as_str());
        property(&mut root, "min-improvement", metric.min_improvement);
        let source = match &metric.source {
            MetricSource::Gate { gate } => text("from-gate", gate),
            MetricSource::Field { path, subject } => node("field", [path.as_str().into(), subject.as_str().into()]),
            MetricSource::Exec { command, host, workspace, environment, time_limit_ms } => {
                block(text("exec", command), vec![text("host", host), text("workspace", workspace),
                    block(node("env", []), environment.keys().map(|name| text(name, "<redacted>")).collect()),
                    duration("time-limit", *time_limit_ms)])
            }
        };
        children.push(block(root, vec![source]));
    }
    let mut stops = Vec::new();
    if let Some(metric) = &stop.plateau_metric {
        let mut plateau = node("plateau", []);
        property(&mut plateau, "metric", metric.as_str());
        property(&mut plateau, "rounds", i128::from(stop.plateau_rounds.ok_or_else(||
            St3Error::new("unrenderable-mission", "plateau metric has no round count"))?));
        stops.push(plateau);
    } else if stop.plateau_rounds.is_some() {
        return Err(St3Error::new("unrenderable-mission", "plateau rounds have no metric"));
    }
    for (name, value) in [("repeated-failure", stop.repeated_failure.map(u64::from)),
        ("no-progress", stop.no_progress.map(u64::from)), ("token-budget", stop.token_budget)] {
        if let Some(value) = value { stops.push(node(name, [i128::from(value).into()])); }
    }
    if !stops.is_empty() { children.push(block(node("stop", []), stops)); }
    if !until.is_empty() {
        children.push(block(node("until", []), until.iter().map(render_gate_node).collect::<Result<_, _>>()?));
    }
    if let Some(candidates) = candidates {
        let mut root = node("candidates", [i128::from(candidates.count).into()]);
        property(&mut root, "max-parallel", i128::from(candidates.max_parallel));
        let select = match &candidates.select {
            LoopCandidateSelector::Metric { metric } => {
                let mut select = node("select", []);
                property(&mut select, "metric", metric.as_str());
                select
            }
            LoopCandidateSelector::Llm { gate } | LoopCandidateSelector::Human { gate } =>
                block(node("select", []), vec![render_gate_node(gate)?]),
        };
        children.push(block(root, vec![select]));
    }
    children.push(block(node("round", []), mission(round)?));
    if let Some(metric) = keep_metric {
        let mut keep = node("keep-if", []);
        property(&mut keep, "metric", metric.as_str());
        children.push(keep);
    }
    for (name, branch) in [("on-keep", on_keep), ("on-discard", on_discard)] {
        if let Some(branch) = branch { children.push(block(node(name, []), mission(branch)?)); }
    }
    let mut exhausted = vec![match on_exhausted {
        LoopExhaustionSpec::Fail => node("fail", []),
        LoopExhaustionSpec::Succeed => node("succeed", []),
        LoopExhaustionSpec::Human { gate } => render_gate_node(gate)?,
    }];
    if let Some(attention) = exhaustion_attention {
        exhausted.push(block(text("attention", &attention.title), vec![text("reviewer", &attention.reviewer),
            text("severity", &attention.severity)]));
    }
    children.push(block(node("on-exhausted", []), exhausted));
    Ok(block(root, children))
}

/// Detect legacy shapes through the entire published closure, including keep/discard branches.
pub fn has_legacy_loop_shapes(mission: &MissionSpec) -> bool {
    mission.steps.values().any(|step| {
        step.nested_mission.as_deref().is_some_and(has_legacy_loop_shapes)
            || step.loop_spec.as_deref().is_some_and(|spec| {
                spec.for_each.is_some() || spec.candidates.is_some() || has_legacy_loop_shapes(&spec.round)
                    || spec.on_keep.as_deref().is_some_and(has_legacy_loop_shapes)
                    || spec.on_discard.as_deref().is_some_and(has_legacy_loop_shapes)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdl::KdlDocument;

    fn fixture() -> MissionSpec {
        let source = r#"mission "test" state="ready" { goal "test";
          loop "retry" timeout="20s" { max-rounds 3;
            metric "quality" direction="higher" min-improvement=0.5 { from-gate "ready"; }
            metric "score" direction="lower" { field "score" "resource/test"; }
            metric "exec" direction="higher" { exec "echo 1" { host "node"; workspace "."; env { TOKEN "secret"; }; time-limit "1s"; }; }
            stop { plateau metric="quality" rounds=2; repeated-failure 2; no-progress 3; token-budget 100; }
            until { gate "ready" { exists "file/node:/test"; }; }
            round { goal "round"; completion { when "all-steps-exhausted"; }; }
            keep-if metric="quality"
            on-keep { goal "keep"; completion { when "all-steps-exhausted"; }; }
            on-discard { goal "discard"; completion { when "all-steps-exhausted"; }; }
            on-exhausted { fail; attention "stopped" { reviewer "person/test"; severity "warning"; }; }
          }
        }"#;
        crate::mission::parse_missions(&source.parse().unwrap(), "node").unwrap().remove("test").unwrap()
    }

    fn body(mission: &MissionSpec) -> Result<Vec<KdlNode>, St3Error> {
        Ok(vec![text("goal", &mission.goals[1]), block(node("completion", []),
            vec![text("when", "all-steps-exhausted")])])
    }

    #[test]
    fn canonical_loop_policy_round_trips_and_redacts_metric_env() {
        let mission = fixture();
        let spec = mission.steps["retry"].loop_spec.as_ref().unwrap();
        let rendered = render_loop_node(spec, &[], &body).unwrap();
        let mut document: KdlDocument = r#"mission "test" state="ready" { goal "test"; }"#.parse().unwrap();
        document.nodes_mut()[0].ensure_children().nodes_mut().push(rendered);
        let reparsed = crate::mission::parse_missions(&document, "node").unwrap().remove("test").unwrap();
        let mut expected = (**spec).clone();
        if let MetricSource::Exec { environment, .. } = &mut expected.metrics[2].source {
            environment.insert("TOKEN".into(), "<redacted>".into());
        }
        assert!(!document.to_string().contains("secret"));
        assert_eq!(*reparsed.steps["retry"].loop_spec.as_ref().unwrap(), Box::new(expected));
    }

    #[test]
    fn legacy_collection_shapes_are_explicit_and_flagged() {
        let mut mission = fixture();
        let spec = mission.steps.get_mut("retry").unwrap().loop_spec.as_mut().unwrap();
        spec.for_each = Some(crate::model::LoopForEachSpec { resource: "resource/items".into(), field: "values".into(), max_parallel: 2 });
        let rendered = render_loop_node(spec, &[], &body).unwrap();
        assert_eq!(rendered.get("for-each").unwrap().as_string(), Some("resource/items"));
        assert_eq!(rendered.get("field").unwrap().as_string(), Some("values"));
        assert_eq!(rendered.get("max-parallel").unwrap().as_integer(), Some(2));
        assert!(rendered.to_string().starts_with("// legacy loop shape; the current parser rejects it\n"));
        assert!(has_legacy_loop_shapes(&mission));
    }

    #[test]
    fn legacy_candidate_selectors_match_last_accepted_syntax() {
        use crate::model::{GateSpec, LoopCandidateSelector, LoopCandidatesSpec};
        for (selector, golden) in [
            (LoopCandidateSelector::Metric { metric: "quality".into() }, r#"candidates 3 max-parallel=2 { select metric="quality"; }"#),
            (LoopCandidateSelector::Human { gate: GateSpec::Human { name: "pick".into(), reviewer: "person/test".into(), mode: Some("approve".into()), question: None, review_targets: vec![] } },
                r#"candidates 3 max-parallel=2 { select { gate "pick" type="human" mode="approve" { reviewer "person/test"; }; }; }"#),
            (LoopCandidateSelector::Llm { gate: GateSpec::Llm { name: "pick".into(), model: "model".into(), host: "node".into(), workspace: ".".into(), tools: vec!["shell".into()], environment: Default::default(), token_budget: 12, time_limit_ms: 1000, prompt: "choose".into() } },
                r#"candidates 3 max-parallel=2 { select { gate "pick" type="llm" { host "node"; workspace "."; env {}; time-limit "1000ms"; model "model"; tools "shell"; token-budget 12; prompt "choose"; }; }; }"#),
        ] {
            let mut mission = fixture();
            let spec = mission.steps.get_mut("retry").unwrap().loop_spec.as_mut().unwrap();
            spec.candidates = Some(LoopCandidatesSpec { count: 3, max_parallel: 2, select: selector });
            let rendered = render_loop_node(spec, &[], &body).unwrap();
            let mut actual = rendered.children().unwrap().nodes().iter().find(|n| n.name().value() == "candidates").unwrap().clone();
            actual.clear_format_recursive();
            actual.autoformat();
            let mut expected: KdlNode = golden.parse().unwrap();
            expected.clear_format_recursive();
            expected.autoformat();
            assert_eq!(actual.to_string(), expected.to_string());
            assert!(has_legacy_loop_shapes(&mission));
            let mut document: KdlDocument = r#"mission "test" state="ready" { goal "test"; }"#.parse().unwrap();
            document.nodes_mut()[0].ensure_children().nodes_mut().push(rendered);
            assert_eq!(crate::mission::parse_missions(&document, "node").unwrap_err().code, "removed-loop-shape");
        }
    }
}
