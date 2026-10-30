use super::*;
use crate::model::IntentInput;

fn objectives() -> BTreeSet<String> {
    BTreeSet::from(["merge-latency/p95".into(), "flakes".into()])
}

fn row(id: &str, status: &str) -> Value {
    json!({
        "id": id, "kind": "objective", "status": status,
        "target": "p95 ≤ 10 min", "current": "p95 26 min over 40 runs",
        "explain": format!("{id} explained."),
    })
}

fn with_fault_agent(store: &Store) {
    let source = "version 2\nagent \"node.ops\" { command \"true\"; handles-faults }\n";
    let intent = crate::graph::parse_intent(source, "node").unwrap();
    let preview = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    store.apply(&intent, &preview.subject_tokens, "objective-faults-owner").unwrap();
    assert_eq!(store.fleet_fault_agent().unwrap().as_deref(), Some("agent/node.ops"));
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
fn a_missed_ci_objective_is_one_fault_for_the_fault_agent_until_it_is_met_again() {
    let store = Store::open_memory("node").unwrap();
    with_fault_agent(&store);
    let missed = [row("merge-latency/p95", "missed"), row("flakes", "met"), row("transaction/write", "missed")];

    let first = check(&store, &missed, &objectives()).unwrap();
    assert_eq!(first, ObjectiveFaults { opened: 1, ..ObjectiveFaults::default() }, "only CI objectives open faults");
    let faults = open(&store);
    let episode = "slo-objective/node/merge-latency/p95";
    assert_eq!(faults.keys().collect::<Vec<_>>(), [episode]);
    let claim = store.claim_by_id(&faults[episode][0]).unwrap().unwrap();
    assert_eq!((claim.subject.as_str(), claim.kind.as_str()), ("daemon/node", "operational.failure"));
    let fields = &claim.body["fields"];
    assert_eq!(fields["condition"], CONDITION);
    assert_eq!(fields["reviewer"], "agent/node.ops", "the fault agent, never a person");
    assert_eq!(fields["severity"], "warning");
    assert_eq!(fields["targets"], json!(["daemon/node"]));
    assert_eq!(fields["title"], "CI objective missed: merge-latency/p95 is p95 26 min over 40 runs against p95 ≤ 10 min");
    assert!(fields["reason"].as_str().unwrap().starts_with("merge-latency/p95 explained."));
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
    assert_eq!(check(&store, &missed, &objectives()).unwrap(), ObjectiveFaults::default());
    assert_eq!(check(&store, &[row("merge-latency/p95", "no-data")], &objectives()).unwrap(), ObjectiveFaults::default());
    assert_eq!(open(&store).len(), 1);

    // Met again, it ends; missed after that, it is a new failure in the same episode.
    let met = [row("merge-latency/p95", "met")];
    assert_eq!(check(&store, &met, &objectives()).unwrap().recovered, 1);
    assert!(open(&store).is_empty());
    assert_eq!(store.operational_failure(episode).unwrap().unwrap().status, "resolved");
    assert_eq!(check(&store, &met, &objectives()).unwrap(), ObjectiveFaults::default());
    assert_eq!(check(&store, &missed, &objectives()).unwrap().opened, 1);
    let again = open(&store);
    assert_eq!(again.len(), 1);
    assert_ne!(again[episode], faults[episode]);
}

#[test]
fn without_a_fault_agent_a_missed_objective_opens_nothing() {
    let store = Store::open_memory("node").unwrap();
    let checked = check(&store, &[row("flakes", "missed")], &objectives()).unwrap();
    assert_eq!(checked, ObjectiveFaults { unowned: 1, ..ObjectiveFaults::default() });
    assert!(open(&store).is_empty());
}

#[test]
fn the_ci_objectives_are_the_declared_durations_and_counts() {
    let ids = ci_objectives(crate::slo::targets());
    assert_eq!(
        ids,
        BTreeSet::from(["ci-critical-path/p95", "flakes", "merge-latency/p50", "merge-latency/p95"].map(String::from)),
    );
}
