use super::*;
use crate::model::IntentInput;

const ROLLOUT: &str = "ops/change-rollout/p95";
const THROUGHPUT: &str = "factory/work-throughput";

fn objectives() -> BTreeSet<String> {
    BTreeSet::from([ROLLOUT.into(), THROUGHPUT.into()])
}

fn row(id: &str, status: &str) -> Value {
    json!({
        "id": id, "kind": "objective", "status": status,
        "target": "p95 ≤ 10 min", "current": "p95 26 min over 40 samples",
        "explain": format!("{id} explained."),
    })
}

fn response(reviewer: &str) -> ObjectiveResponse<'_> {
    ObjectiveResponse { reviewer, condition: "change-rollout-slo-missed", title: "ops/change-rollout" }
}

fn check(store: &Store, rows: &[Value]) -> ObjectiveFaults {
    check_with_response(store, rows, &objectives(), response("agent/node.ops")).unwrap()
}

fn with_reviewer(store: &Store) {
    let source = "version 2\nagent \"node.ops\" { command \"true\"; handles-faults }\n";
    let intent = crate::graph::parse_intent(source, "node").unwrap();
    let preview = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply(&intent, &preview.subject_tokens, "objective-faults-owner").unwrap();
    assert!(store.fault_reviewer_is_declared("agent/node.ops").unwrap());
}

fn open(store: &Store) -> BTreeMap<String, Vec<String>> {
    objective_faults(store, "daemon/node")
        .unwrap()
        .into_iter()
        .filter(|(_, episode)| !episode.open.is_empty())
        .map(|(name, episode)| (name, episode.open))
        .collect()
}

#[test]
fn a_missed_objective_is_one_fault_for_its_reviewer_until_it_is_met_again() {
    let store = Store::open_memory("node").unwrap();
    with_reviewer(&store);
    let missed = [row(ROLLOUT, "missed"), row(THROUGHPUT, "met"), row("transaction/write", "missed")];

    let first = check(&store, &missed);
    assert_eq!(first, ObjectiveFaults { opened: 1, ..ObjectiveFaults::default() }, "only the objectives asked about open faults");
    let faults = open(&store);
    let episode = "slo-objective/node/ops/change-rollout/p95";
    assert_eq!(faults.keys().collect::<Vec<_>>(), [episode]);
    let claim = store.claim_by_id(&faults[episode][0]).unwrap().unwrap();
    assert_eq!((claim.subject.as_str(), claim.kind.as_str()), ("daemon/node", "operational.failure"));
    let fields = &claim.body["fields"];
    assert_eq!(fields["condition"], "change-rollout-slo-missed");
    assert_eq!(fields["reviewer"], "agent/node.ops", "the reviewer, never a person");
    assert_eq!(fields["severity"], "warning");
    assert_eq!(fields["targets"], json!(["daemon/node"]));
    assert_eq!(fields["title"], "ops/change-rollout objective missed: ops/change-rollout/p95 is p95 26 min over 40 samples against p95 ≤ 10 min");
    assert!(fields["reason"].as_str().unwrap().starts_with("ops/change-rollout/p95 explained."));
    let failure = store.operational_failure(episode).unwrap().unwrap();
    assert_eq!((failure.status.as_str(), failure.reviewer.as_str()), ("pending", "agent/node.ops"));
    let now = crate::gate_check::now_ms() + 1;
    let fault = store
        .fault_snapshot(now)
        .unwrap()
        .into_iter()
        .find(|fault| fault.item.subject == "daemon/node")
        .expect("the missed objective is a fault");
    assert_eq!((fault.owner.as_deref(), fault.item.priority.as_str()), (Some("agent/node.ops"), "normal"));
    assert!(store.attention_snapshot(Some("person/operator"), now).unwrap().is_empty(), "no person sees it");

    // Missed again, it stays the one episode; with no data it neither opens nor ends.
    assert_eq!(check(&store, &missed), ObjectiveFaults::default());
    assert_eq!(check(&store, &[row(ROLLOUT, "no-data")]), ObjectiveFaults::default());
    assert_eq!(open(&store).len(), 1);

    // Met again, it ends; missed after that, it is a new failure in the same episode.
    let met = [row(ROLLOUT, "met")];
    assert_eq!(check(&store, &met).recovered, 1);
    assert!(open(&store).is_empty());
    assert_eq!(store.operational_failure(episode).unwrap().unwrap().status, "resolved");
    assert_eq!(check(&store, &met), ObjectiveFaults::default());
    assert_eq!(check(&store, &missed).opened, 1);
    let again = open(&store);
    assert_eq!(again.len(), 1);
    assert_ne!(again[episode], faults[episode]);
}

#[test]
fn an_undeclared_reviewer_never_falls_back_to_the_fleet_fault_agent() {
    let store = Store::open_memory("node").unwrap();
    with_reviewer(&store);
    assert_eq!(store.fleet_fault_agent().unwrap().as_deref(), Some("agent/node.ops"));
    let result = check_with_response(&store, &[row(THROUGHPUT, "missed")], &objectives(), response("agent/missing")).unwrap();
    assert_eq!(result, ObjectiveFaults { unowned: 1, ..ObjectiveFaults::default() });
    assert!(open(&store).is_empty());
}

#[test]
fn a_row_no_longer_judged_ends_its_fault_and_a_judged_or_foreign_one_keeps_it() {
    let store = Store::open_memory("node").unwrap();
    with_reviewer(&store);
    let missed = [row(ROLLOUT, "missed"), row(THROUGHPUT, "missed")];
    assert_eq!(check(&store, &missed).opened, 2);
    let judged = BTreeSet::from([ROLLOUT.to_owned()]);
    // The throughput row is no longer judged; the rollout row is, so only a met row ends it.
    assert_eq!(end_unjudged(&store, |id| objectives().contains(id), &judged).unwrap(), 1);
    assert_eq!(open(&store).keys().collect::<Vec<_>>(), ["slo-objective/node/ops/change-rollout/p95"]);
    assert_eq!(store.operational_failure("slo-objective/node/factory/work-throughput").unwrap().unwrap().status, "resolved");
    // Nothing left to end, and a row the caller does not own is never ended.
    assert_eq!(end_unjudged(&store, |id| objectives().contains(id), &judged).unwrap(), 0);
    assert_eq!(end_unjudged(&store, |_| false, &BTreeSet::new()).unwrap(), 0);
    assert_eq!(open(&store).len(), 1);
}

#[test]
fn an_upgrade_ends_the_retired_ci_objectives_faults_and_keeps_every_other() {
    let store = Store::open_memory("node").unwrap();
    with_reviewer(&store);
    // Faults an older build opened for its shipped CI objectives, beside a declared objective's
    // and the successor's under fractal/.
    let retired = ["merge-latency/p95", "flakes"];
    let kept = [ROLLOUT, "fractal/merge-latency/p95"];
    let ids: BTreeSet<String> = retired.iter().chain(&kept).map(|id| (*id).to_owned()).collect();
    let rows: Vec<Value> = ids.iter().map(|id| row(id, "missed")).collect();
    let old = ObjectiveResponse { reviewer: "agent/node.ops", condition: "slo-objective-missed", title: "CI" };
    assert_eq!(check_with_response(&store, &rows, &ids, old).unwrap().opened, 4);

    assert_eq!(end_retired_ci_objectives(&store).unwrap(), 2);
    let still: Vec<String> = kept.iter().map(|id| format!("slo-objective/node/{id}")).collect();
    assert_eq!(open(&store).into_keys().collect::<BTreeSet<_>>(), still.into_iter().collect());
    for id in retired {
        let episode = format!("slo-objective/node/{id}");
        assert_eq!(store.operational_failure(&episode).unwrap().unwrap().status, "resolved", "{episode}");
    }
    let recovered = store.claims_for("daemon/node", Some("operational.recovered")).unwrap();
    let reason = recovered
        .iter()
        .find(|claim| claim.body["fields"]["episode"] == "slo-objective/node/flakes")
        .map(|claim| claim.body["fields"]["reason"].as_str().unwrap().to_owned())
        .unwrap();
    assert!(reason.starts_with("The objective flakes is retired"), "{reason}");
    // Idempotent: a second start ends nothing.
    assert_eq!(end_retired_ci_objectives(&store).unwrap(), 0);
    assert_eq!(open(&store).len(), 2);
}
