//! Loop reads that wanted one claim from a round history now read only up to it. Each test
//! checks the bounded read decides as the whole history did.
use super::*;
use crate::model::{MetricSource, MetricSpec};

const LOOP: &str = "loop-run/invented-generation/review";

fn kept_loop() -> (Arc<Store>, Reconciler<FakeRuntime>, LoopSpec) {
    let store = Arc::new(Store::open_memory("node").unwrap());
    apply_source(
        &store,
        r#"
version 2
resource "result" { kind "custom.test.loop-result" }
mission "improve" state="ready" {
  goal "Improve a result."
  completion { when "all-steps-exhausted" }
  loop "review" {
    max-rounds 6
    until { gate "ready" { field "state" "resource/result" is "ready" } }
    round {
      completion { when "all-steps-exhausted" }
      step "work" {
        agentless
        goal "Complete round ${loop.round}."
      }
    }
  }
}
"#,
        "bounded-loop-reads",
    );
    let mut spec = store
        .mission_spec("improve", None)
        .unwrap()
        .unwrap()
        .steps
        .values()
        .find_map(|step| step.loop_spec.clone())
        .map(|spec| *spec)
        .unwrap();
    spec.keep_metric = Some("quality".into());
    spec.metrics = vec![MetricSpec {
        name: "quality".into(),
        direction: "higher".into(),
        min_improvement: 0.5,
        source: MetricSource::Field { subject: "resource/result".into(), path: "score".into() },
    }];
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(FakeRuntime::default()),
        "node".into(),
        Arc::new(Notify::new()),
    );
    (store, reconciler, spec)
}

fn result(store: &Store, round: u64, status: &str, quality: Option<f64>) {
    let mut fields = BTreeMap::from([
        ("round".into(), Value::from(round)),
        ("status".into(), Value::from(status)),
        ("mission_run".into(), Value::from(format!("mission-run/invented-round-{round}"))),
    ]);
    if let Some(quality) = quality {
        fields.insert("metrics".into(), serde_json::json!({ "quality": quality }));
    }
    store
        .append_claim(&ClaimInput {
            subject: LOOP.into(),
            kind: "loop.round-result".into(),
            actor: None,
            fields,
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

#[test]
fn a_kept_round_compares_with_the_newest_completed_result_that_has_the_metric() {
    let (store, reconciler, spec) = kept_loop();
    // Nothing to compare with yet: any value is kept.
    let current = |quality: f64| BTreeMap::from([("quality".to_owned(), quality)]);
    assert!(reconciler.loop_round_improves(LOOP, &spec, &current(0.0)).unwrap());
    result(&store, 1, "completed", Some(1.0));
    result(&store, 2, "completed", Some(3.0));
    // Newer, but a failed round and a completed one without the metric do not count.
    result(&store, 3, "failed", Some(9.0));
    result(&store, 4, "completed", None);
    let whole = store
        .claims_for(LOOP, Some("loop.round-result"))
        .unwrap()
        .into_iter()
        .filter(|claim| claim.body.pointer("/fields/status").and_then(Value::as_str) == Some("completed"))
        .filter_map(|claim| claim.body.pointer("/fields/metrics/quality").and_then(Value::as_f64))
        .next_back();
    assert_eq!(whole, Some(3.0));
    assert!(!reconciler.loop_round_improves(LOOP, &spec, &current(3.4)).unwrap());
    assert!(reconciler.loop_round_improves(LOOP, &spec, &current(3.5)).unwrap());
}
