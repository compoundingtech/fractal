use super::*;

const SHIPPED: &str = /* sqlite-large-value: allow-for-now lv-ha01213f3c4b5 owner=fractal_kernel::objective_tests TODO=trace-statement-and-value-representation */ include_str!("../../../slo/objectives.kdl");

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
    assert_eq!(objectives.len(), 21);
    assert!(objectives.iter().all(|objective| !objective.declarable()));
    let merge = objectives.iter().find(|objective| objective.name == "merge-latency").unwrap();
    assert_eq!(merge.indicator, Indicator::CiDuration { series: "ci/merge-queue".into() });
    assert_eq!(
        merge.targets.iter().map(|target| (target.percentile(), target.within_ms())).collect::<Vec<_>>(),
        [(Some(50), Some(600_000)), (Some(95), Some(600_000))],
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
        ("objective \"db\" { about \"x\"; indicator \"database\"; target \"live-gb\" max=1.0; target \"growth-mb-per-day\" max=1; window \"1d\" }".into(), "only an interval takes a `window`"),
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
