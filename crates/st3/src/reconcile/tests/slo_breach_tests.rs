//! Actual field-gate evaluation used by the old-registry/prefix store controls.
use super::*;

pub(crate) fn gate_passes(store: Arc<Store>, episode: &str) -> bool {
    let reconciler = Reconciler::new(store, Arc::new(FakeRuntime::default()), "prefix".into(), Arc::new(Notify::new()));
    let stage = GateContext { subject: "step-run/prefix/dispatch-ready".into(), name: "dispatch-ready".into(),
        started_at_unix_ms: 0, attempt: 1, run: format!("mission-run/slo/{episode}"), generation: "run-generation/prefix".into(), eval: false };
    let gate = GateSpec::Field { name: "execution admitted".into(), path: "dispatch_state".into(),
        subject: format!("resource/slo-breach/{episode}"), operator: "is".into(), value: json!("ready") };
    matches!(reconciler.evaluate_gate(&stage, &gate).unwrap(), GateOutcome::Pass)
}
