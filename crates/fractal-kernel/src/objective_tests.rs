use super::*;

const SHIPPED: &str = /* sqlite-large-value: allow-for-now lv-ha01213f3c4b5 owner=fractal_kernel::objective_tests TODO=trace-statement-and-value-representation */ include_str!("../../../slo/objectives.kdl");
const FRACTAL_CI: &str = /* sqlite-large-value: allow-for-now lv-h44b5714bd710 owner=fractal_kernel::objective_tests TODO=trace-statement-and-value-representation */ include_str!("../../../slo/fractal-ci.kdl");

fn one(text: &str) -> Result<Objective, FractalError> {
    parse_document(&format!("version 2\n{text}")).map(|mut objectives| objectives.remove(0))
}

const INTERVAL: &str = r#"objective "ops/change-rollout" {
  about "A change, from ready to served by every member"
  indicator "interval" {
    subject-prefix "custom/acme/change/"
    start "custom.acme.change-ready" at="ready_at"
    expect "custom.acme.change-merged" set="hosts" at="merged_at"
    stop "custom.acme.change-applied" member="host" at="applied_at"
    cancel "custom.acme.change-abandoned"
    pause "custom.acme.change-paused" resume="custom.acme.change-resumed"
    label "class"
    label "member"
    exclude "exclusion" "deferred-launch" "deferred-runtime"
  }
  window "7d" min-samples=20
  target "p50" within="60s"
  target "p95" within="5m"
  on-breach "fault" owner="agent/ops/release-manager" backup="agent/ops/observability" condition="change-rollout-slo-missed"
}"#;

#[test]
fn the_shipped_objectives_parse_and_none_is_declarable() {
    let objectives = parse_document(SHIPPED).unwrap();
    assert_eq!(objectives.len(), 18);
    assert!(objectives.iter().all(|objective| !objective.declarable()));
    let statement = objectives.iter().find(|objective| objective.name == "sql-statement").unwrap();
    assert_eq!(statement.indicator, Indicator::Statement);
    assert_eq!(
        statement.targets.iter().map(|target| (target.stat.as_str(), target.within_ms())).collect::<Vec<_>>(),
        [("p99", Some(10)), ("max", Some(100))],
    );
    let read = objectives.iter().find(|objective| objective.name == "person-read").unwrap();
    assert_eq!(read.target("p99", true).and_then(Target::within_ms), Some(300));
}

#[test]
fn an_interval_reads_every_role() {
    let objective = one(INTERVAL).unwrap();
    assert!(objective.declarable());
    assert_eq!(objective.window, Some(Window { ms: 7 * 86_400_000, min_samples: 20 }));
    let Indicator::Interval(interval) = &objective.indicator else { panic!() };
    assert_eq!(interval.subject_prefix, "custom/acme/change/");
    assert_eq!(interval.start, Event { kind: "custom.acme.change-ready".into(), at: Some("ready_at".into()) });
    assert_eq!(interval.expect.as_ref().map(|(_, set)| set.as_str()), Some("hosts"));
    assert_eq!(interval.stop.1.as_deref(), Some("host"));
    assert_eq!(interval.labels, ["class", "member"]);
    assert_eq!(interval.excludes, [Exclude { field: "exclusion".into(), values: vec!["deferred-launch".into(), "deferred-runtime".into()] }]);
    assert_eq!(interval.kinds().len(), 6);
    assert_eq!(
        objective.on_breach,
        Some(Breach {
            owner: "agent/ops/release-manager".into(),
            backup: Some("agent/ops/observability".into()),
            condition: "change-rollout-slo-missed".into(),
        })
    );
    assert_eq!(percentile_row(&objective.name, 95), "ops/change-rollout/p95");
}

#[test]
fn the_expect_kind_may_also_start_the_sample() {
    let objective = one(&INTERVAL.replace("start \"custom.acme.change-ready\" at=\"ready_at\"", "start \"custom.acme.change-merged\" at=\"merged_at\"")).unwrap();
    let Indicator::Interval(interval) = &objective.indicator else { panic!() };
    assert_eq!(
        interval.kinds(),
        ["custom.acme.change-merged", "custom.acme.change-applied", "custom.acme.change-abandoned", "custom.acme.change-paused", "custom.acme.change-resumed"]
    );
}

fn only(field: &str, values: &[&str]) -> Only {
    Only { field: field.into(), values: values.iter().map(|value| (*value).to_owned()).collect() }
}

const RUN: &str = r#"objective "ci/merge-latency" {
  about "A merge group's run, from created to completed"
  indicator "interval" {
    subject-prefix "github-workflow-run/acme/repo/"
    start "github.workflow-run" at="created_at"
    stop "github.workflow-run" at="completed_at"
    only "event" "merge_group"
    only "attempt" "1" "2"
  }
  window "1d"
  target "p95" within="10m"
}"#;

#[test]
fn one_claim_may_be_the_whole_sample_and_only_keeps_the_matching_ones() {
    let objective = one(RUN).unwrap();
    let Indicator::Interval(interval) = &objective.indicator else { panic!() };
    assert_eq!(interval.start, Event { kind: "github.workflow-run".into(), at: Some("created_at".into()) });
    assert_eq!(interval.stop, (Event { kind: "github.workflow-run".into(), at: Some("completed_at".into()) }, None));
    assert_eq!(interval.kinds(), ["github.workflow-run"], "one kind, read once");
    assert_eq!(interval.only, [only("event", &["merge_group"]), only("attempt", &["1", "2"])]);
    let refused = |replace: &str, with: &str| {
        assert!(RUN.contains(replace), "{replace}");
        one(&RUN.replace(replace, with)).unwrap_err().message
    };
    for (message, says) in [
        (refused("at=\"completed_at\"", "at=\"created_at\""), "different `at=` fields"),
        (refused(" at=\"completed_at\"", ""), "different `at=` fields"),
        (refused(" at=\"created_at\"", ""), "different `at=` fields"),
        (refused("at=\"completed_at\"", "at=\"completed_at\" member=\"host\""), "takes no `expect` or `member=`"),
        (
            refused(
                "    stop \"github.workflow-run\" at=\"completed_at\"\n",
                "    expect \"github.workflow-run\" set=\"hosts\" at=\"created_at\"\n    stop \"github.workflow-run\" member=\"host\" at=\"completed_at\"\n",
            ),
            "takes no `expect` or `member=`",
        ),
        (refused("only \"attempt\" \"1\" \"2\"", "only \"event\" \"pull_request\""), "`only \"event\"` is repeated"),
        (refused("only \"attempt\" \"1\" \"2\"", "only \"attempt\""), "`only` needs a field and at least one value"),
        (refused("only \"attempt\" \"1\" \"2\"", "only \"attempt\" \"\""), "`only` needs a field and at least one value"),
        (refused("only \"attempt\" \"1\" \"2\"", "only \"attempt\" 1"), "`only` needs a field and at least one value"),
    ] {
        assert!(message.contains(says), "{says:?} not in {message:?}");
    }
}

#[test]
fn the_fractal_ci_objectives_are_declarable_over_workflow_run_claims() {
    let objectives = parse_document(FRACTAL_CI).unwrap();
    assert_eq!(
        objectives.iter().map(|objective| objective.name.as_str()).collect::<Vec<_>>(),
        ["fractal/merge-latency", "fractal/ci-critical-path", "fractal/flakes"]
    );
    assert!(objectives.iter().all(Objective::declarable));
    let run = |at: &str| Event { kind: "github.workflow-run".into(), at: Some(at.into()) };
    for (objective, start, event) in [(&objectives[0], "created_at", "merge_group"), (&objectives[1], "started_at", "pull_request")] {
        let Indicator::Interval(interval) = &objective.indicator else { panic!("{} is an interval", objective.name) };
        assert_eq!(interval.subject_prefix, "github-workflow-run/compoundingtech/fractal/");
        assert_eq!((&interval.start, &interval.stop), (&run(start), &(run("completed_at"), None)));
        assert_eq!(interval.kinds(), ["github.workflow-run"]);
        assert_eq!(
            interval.only,
            [only("workflow", &["Workspace CI"]), only("event", &[event]), only("conclusion", &["success"]), only("attempt", &["1"])]
        );
    }
    let Indicator::Count(count) = &objectives[2].indicator else { panic!("flakes is a count") };
    assert_eq!((count.event.as_str(), count.at.as_str()), ("github.workflow-run", "completed_at"));
    assert_eq!(count.only, [only("workflow", &["Workspace CI"])]);
    assert_eq!(count.excludes, [Exclude { field: "flakes".into(), values: vec!["0".into()] }]);
    assert_eq!(objectives[2].target("per-hour", false).map(|target| target.bound), Some(Bound::Max(0.0)));
}

#[test]
fn a_nonsensical_objective_is_refused_never_defaulted() {
    let refused = |text: &str| one(text).unwrap_err().message;
    let interval = |replace: &str, with: &str| INTERVAL.replace(replace, with);
    for (text, says) in [
        (interval("  window \"7d\" min-samples=20\n", ""), "needs a `window`"),
        (interval("target \"p95\" within=\"5m\"", "target \"p95\" within=\"5m\"\n  target \"p95\" within=\"9m\""), "repeats target"),
        (interval("target \"p95\"", "target \"p100\""), "no target `p100`"),
        (interval("target \"p95\"", "target \"p05\""), "no target `p05`"),
        (interval(" member=\"host\"", ""), "needs `member=FIELD`"),
        (interval("cancel \"custom.acme.change-abandoned\"", "cancel \"custom.acme.change-ready\""), "two roles"),
        (interval("backup=\"agent/ops/observability\"", "backup=\"agent/ops/release-manager\""), "other than the owner"),
        (interval("owner=\"agent/ops/release-manager\"", "owner=\"person/ada\""), "needs `owner="),
        (interval("subject-prefix \"custom/acme/change/\"", "subject-prefix \"custom/acme/change\""), "end with `/`"),
        (interval("    label \"class\"\n", "    label \"class\"\n    color \"red\"\n"), "unknown child `color`"),
        (interval("about \"A change, from ready to served by every member\"", "about \" \""), "`about` is empty"),
        ("objective \"cpu\" { about \"x\"; indicator \"cpu\" { idle-min-minutes 10; load-max-cores 2.0 }; target \"cores\" max=1.0 }".into(), "needs target `idle-cores`"),
        ("objective \"db\" { about \"x\"; indicator \"database\"; target \"live-gb\" max=1.0; target \"growth-mb-per-day\" max=1; window \"1d\" }".into(), "only an interval or a count takes a `window`"),
        ("objective \"r\" { about \"x\"; indicator \"latency\" { path \"GET /v1/x\" }; target \"p99\" within=\"300ms\"; target \"p99\" within=\"200ms\" scope=\"remote\" }".into(), "remote latency is below"),
        ("objective \"Bad\" { about \"x\" }".into(), "lowercase words"),
        ("objective \"ops//x\" { about \"x\" }".into(), "lowercase words"),
    ] {
        let message = refused(&text);
        assert!(message.contains(says), "{says:?} not in {message:?}");
    }
    assert!(parse_document(&format!("version 2\n{INTERVAL}\n{INTERVAL}")).unwrap_err().message.contains("declared twice"));
    assert!(!one(&INTERVAL.replace("ops/change-rollout", "change-rollout")).unwrap().declarable());
}

const COUNT: &str = r#"objective "factory/work-throughput" {
  about "Factory-authored pull requests merged into the roster's default branches"
  indicator "count" {
    subject-prefix "custom/factory/merged-pr/"
    event "custom.factory.pr-merged" at="merged_at"
    label "repository"
    exclude "author" "renovate" "dependabot"
  }
  window "24h"
  target "per-hour" min=10
  on-breach "fault" owner="agent/factory/manager" condition="work-throughput-low"
}"#;

#[test]
fn a_count_reads_its_event() {
    let objective = one(COUNT).unwrap();
    assert!(objective.declarable());
    assert_eq!(objective.indicator.kind(), "count");
    assert_eq!(objective.window, Some(Window { ms: 86_400_000, min_samples: 1 }));
    let Indicator::Count(count) = &objective.indicator else { panic!() };
    assert_eq!(count.subject_prefix, "custom/factory/merged-pr/");
    assert_eq!((count.event.as_str(), count.at.as_str()), ("custom.factory.pr-merged", "merged_at"));
    assert_eq!(count.labels, ["repository"]);
    assert_eq!(count.excludes, [Exclude { field: "author".into(), values: vec!["renovate".into(), "dependabot".into()] }]);
    assert_eq!(objective.target("per-hour", false).map(|target| target.bound), Some(Bound::Min(10.0)));
    let flakes = one(&COUNT.replace("min=10", "max=0.5")).unwrap();
    assert_eq!(flakes.target("per-hour", false).map(|target| target.bound), Some(Bound::Max(0.5)));
}

#[test]
fn a_nonsensical_count_is_refused_never_defaulted() {
    let count = |replace: &str, with: &str| {
        assert!(COUNT.contains(replace), "{replace}");
        COUNT.replace(replace, with)
    };
    for (text, says) in [
        (count(" at=\"merged_at\"", ""), "`event` needs `at=FIELD`"),
        (count("min=10", "min=10 max=20"), "needs one bound"),
        (count("min=10", ""), "needs one bound"),
        (count("min=10", "min=0"), "no target `per-hour`"),
        (count("min=10", "min=-1"), "no target `per-hour`"),
        (count("min=10", "max=-1"), "no target `per-hour`"),
        (count("min=10", "min-percent=10"), "no target `per-hour`"),
        (count("min=10", "within=\"1h\""), "no target `per-hour`"),
        (count("target \"per-hour\"", "target \"p95\""), "no target `p95`"),
        (count("min=10", "min=10\n  target \"per-hour\" max=20"), "repeats target"),
        (count("  window \"24h\"\n", ""), "a `count` indicator needs a `window`"),
        (count("window \"24h\"", "window \"24h\" min-samples=5"), "takes no `min-samples`"),
        (count("    subject-prefix \"custom/factory/merged-pr/\"\n", ""), "needs `subject-prefix`"),
        (count("subject-prefix \"custom/factory/merged-pr/\"", "subject-prefix \"custom/factory/merged-pr\""), "end with `/`"),
        (count("    event \"custom.factory.pr-merged\" at=\"merged_at\"\n", ""), "needs `event`"),
        (count("event \"custom.factory.pr-merged\" at=\"merged_at\"", "event \"custom.factory.pr-merged\" at=\"merged_at\"\n    event \"custom.factory.pr-closed\" at=\"closed_at\""), "repeats `event`"),
        (count("at=\"merged_at\"", "at=\"merged_at\" set=\"hosts\""), "set"),
        (count("    label \"repository\"\n", "    label \"repository\"\n    label \"repository\"\n"), "is repeated"),
        (count("exclude \"author\" \"renovate\" \"dependabot\"", "exclude \"author\""), "`exclude` needs a field"),
        (count("    label \"repository\"\n", "    start \"custom.factory.pr-opened\"\n"), "unknown child `start`"),
    ] {
        let message = one(&text).unwrap_err().message;
        assert!(message.contains(says), "{says:?} not in {message:?}");
    }
    assert!(!one(&COUNT.replace("factory/work-throughput", "throughput")).unwrap().declarable());
}
