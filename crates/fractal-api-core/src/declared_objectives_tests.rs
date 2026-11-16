use super::*;
use crate::model::{ClaimInput, IntentInput};

const MINUTE: u64 = 60_000;
/// The evaluation time; claim times are given relative to it in their fields. It is the clock's,
/// since a cancel or pause is timed by its acceptance.
static NOW: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
});
const READY: &str = "custom.acme.change-ready";
const MERGED: &str = "custom.acme.change-merged";
const APPLIED: &str = "custom.acme.change-applied";
const ABANDONED: &str = "custom.acme.change-abandoned";

fn declaration(extra_interval: &str, window: &str, breach: &str) -> String {
    format!(
        r#"version 2
objective "ops/change-rollout" {{
  about "A change, from ready to every member that serves it"
  indicator "interval" {{
    subject-prefix "custom/acme/change/"
    start "{READY}" at="ready_at"
    expect "{MERGED}" set="hosts" at="merged_at"
    stop "{APPLIED}" member="host" at="applied_at"
    cancel "{ABANDONED}"
{extra_interval}  }}
  window "{window}"
  target "p50" within="60s"
  target "p95" within="5m"
{breach}}}
"#
    )
}

fn apply(store: &Store, source: &str) {
    let intent = crate::graph::parse_intent(source, "node").unwrap();
    let preview = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None }).unwrap();
    // One key per publication: a repeated key returns the first publication's result.
    store.apply(&intent, &preview.subject_tokens, &intent.source_hash).unwrap();
}

fn store_with(source: &str) -> Store {
    let store = Store::open_memory("node").unwrap();
    apply(&store, source);
    store
}

fn claim(store: &Store, change: &str, kind: &str, fields: Value) -> u64 {
    store
        .append_claim(&ClaimInput {
            subject: format!("custom/acme/change/{change}"),
            kind: kind.into(),
            actor: None,
            fields: fields.as_object().unwrap().clone().into_iter().collect(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap()
        .store_index
}

/// A change ready `ago` minutes before NOW, merged for `hosts`, and applied on each `(host, after)`
/// `after` seconds past ready.
fn change(store: &Store, name: &str, ago_ms: u64, hosts: &[&str], applied: &[(&str, u64)], class: &str) {
    let ready = *NOW - ago_ms;
    claim(store, name, READY, json!({"ready_at": ready, "class": class}));
    claim(store, name, MERGED, json!({"merged_at": ready + 1_000, "hosts": hosts}));
    for (host, after_s) in applied {
        claim(store, name, APPLIED, json!({"host": host, "applied_at": ready + after_s * 1_000}));
    }
}

fn evaluate(store: &Store) -> BTreeMap<String, Value> {
    rows(store, *NOW).unwrap().into_iter().map(|row| (row["id"].as_str().unwrap().to_owned(), row)).collect()
}

fn s(text: &str) -> String {
    text.to_owned()
}

#[test]
fn a_change_closes_at_its_last_expected_member_and_rows_have_the_base_keys() {
    let store = store_with(&declaration("", "7d", ""));
    change(&store, "a", 10 * MINUTE, &["hosta", "hostb"], &[("hosta", 40), ("hostb", 55)], "routine");
    let rows = evaluate(&store);
    assert_eq!(rows.keys().cloned().collect::<Vec<_>>(), [s("ops/change-rollout/p50"), s("ops/change-rollout/p95")]);
    let p50 = &rows["ops/change-rollout/p50"];
    let mut keys = p50.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    keys.sort();
    assert_eq!(keys, ["about", "current", "explain", "id", "kind", "remaining_percent", "status", "target", "window"]);
    assert_eq!(p50["kind"], "objective");
    assert_eq!(p50["target"], "p50 ≤ 1 min");
    assert_eq!(p50["current"], "p50 55 s over 1 sample", "the latest member's stop, not the first");
    assert_eq!(p50["window"], "7d");
    assert_eq!((p50["status"].as_str(), p50["remaining_percent"].as_f64()), (Some("met"), Some(100.0)));
    let explain = p50["explain"].as_str().unwrap();
    assert!(explain.contains("Declared as objective/ops/change-rollout at revision "), "{explain}");
    assert!(explain.contains("1 closed, 0 open over 1 min, 0 open within it, 0 cancelled, 0 excluded or left out, 0 unreadable claims."), "{explain}");
    assert!(explain.ends_with("No response is declared: a miss is shown and nothing is delivered."), "{explain}");
}

#[test]
fn an_open_change_over_target_counts_at_its_age_and_a_younger_one_is_left_out() {
    let store = store_with(&declaration("", "7d", ""));
    change(&store, "a", 10 * MINUTE, &["hosta", "hostb"], &[("hosta", 40), ("hostb", 55)], "routine");
    change(&store, "b", 10 * MINUTE, &["hosta"], &[("hosta", 130)], "routine");
    // Merged for two hosts, applied on one, ready 20 minutes ago: open and overdue.
    change(&store, "c", 20 * MINUTE, &["hosta", "hostb"], &[("hosta", 30)], "routine");
    // Ready 30 seconds ago: open, within both targets.
    change(&store, "d", 30_000, &["hosta"], &[], "routine");
    let rows = evaluate(&store);
    let p50 = &rows["ops/change-rollout/p50"];
    // p50 population: 55 s, 130 s, and c at 20 min (over 60 s); d (30 s) is within, left out.
    assert_eq!(p50["current"], "p50 2.2 min over 3 samples (1 open over)");
    assert_eq!(p50["status"], "missed", "2 of 3 over 60 s against 50 in 100");
    let explain = p50["explain"].as_str().unwrap();
    assert!(explain.contains("2 closed, 1 open over 1 min, 1 open within it"), "{explain}");
    assert!(explain.contains("custom/acme/change/c (open 20 min)"), "{explain}");
    let p95 = &rows["ops/change-rollout/p95"];
    // p95 population: 55 s, 130 s and c at 20 min (over 5 min); its nearest rank is c's age.
    assert_eq!(p95["current"], "p95 20 min over 3 samples (1 open over)");
    assert_eq!((p95["status"].as_str(), p95["remaining_percent"].as_f64()), (Some("missed"), Some(0.0)));
}

#[test]
fn a_cancel_ends_an_open_change_but_not_one_that_stopped_first() {
    let store = store_with(&declaration("", "7d", ""));
    change(&store, "open", 20 * MINUTE, &["hosta"], &[], "routine");
    claim(&store, "open", ABANDONED, json!({}));
    // Stopped long before the cancel's acceptance (now, on the real clock).
    change(&store, "closed", 20 * MINUTE, &["hosta"], &[("hosta", 45)], "routine");
    claim(&store, "closed", ABANDONED, json!({}));
    let p50 = &evaluate(&store)["ops/change-rollout/p50"];
    assert_eq!(p50["current"], "p50 45 s over 1 sample");
    assert!(p50["explain"].as_str().unwrap().contains("1 closed, 0 open over 1 min, 0 open within it, 1 cancelled"));
}

#[test]
fn an_empty_expected_set_stops_the_change_at_the_expect_claim() {
    let store = store_with(&declaration("", "7d", ""));
    let ready = *NOW - 10 * MINUTE;
    claim(&store, "noop", READY, json!({"ready_at": ready}));
    claim(&store, "noop", MERGED, json!({"merged_at": ready + 20_000, "hosts": "[]"}));
    let p50 = &evaluate(&store)["ops/change-rollout/p50"];
    assert_eq!(p50["current"], "p50 20 s over 1 sample", "a JSON-encoded empty list");
}

#[test]
fn one_kind_may_start_the_sample_and_name_its_members() {
    // Merge to live: the merge claim both starts the sample and names the hosts it must stop on.
    let source = declaration("", "7d", "").replace(&format!("start \"{READY}\" at=\"ready_at\""), &format!("start \"{MERGED}\" at=\"merged_at\""));
    let store = store_with(&source);
    change(&store, "a", 10 * MINUTE, &["hosta", "hostb"], &[("hosta", 21), ("hostb", 31)], "routine");
    // Merged 20 minutes ago for two hosts and live on one: open and overdue.
    change(&store, "b", 20 * MINUTE, &["hosta", "hostb"], &[("hosta", 11)], "routine");
    let rows = evaluate(&store);
    let p50 = &rows["ops/change-rollout/p50"];
    // a: merged at ready + 1 s, last host at ready + 31 s, so 30 s; b enters at its age since merge.
    assert_eq!(p50["current"], "p50 30 s over 2 samples (1 open over)");
    assert!(p50["explain"].as_str().unwrap().contains("1 closed, 1 open over 1 min, 0 open within it"));
}

#[test]
fn an_excluded_change_is_counted_apart_and_never_in_a_percentile() {
    let store = store_with(&declaration("    exclude \"exclusion\" \"deferred-launch\"\n", "7d", ""));
    change(&store, "a", 10 * MINUTE, &["hosta"], &[("hosta", 40)], "routine");
    let ready = *NOW - 20 * MINUTE;
    claim(&store, "x", READY, json!({"ready_at": ready}));
    claim(&store, "x", MERGED, json!({"merged_at": ready, "hosts": ["hosta"], "exclusion": "deferred-launch"}));
    let p95 = &evaluate(&store)["ops/change-rollout/p95"];
    assert_eq!(p95["current"], "p95 40 s over 1 sample");
    assert!(p95["explain"].as_str().unwrap().contains("0 cancelled, 1 excluded or left out"));
}

#[test]
fn labels_split_rows_by_a_field_and_by_member() {
    let store = store_with(&declaration("    label \"class\"\n    label \"member\"\n", "7d", ""));
    change(&store, "a", 10 * MINUTE, &["hosta", "hostb"], &[("hosta", 40), ("hostb", 55)], "routine");
    change(&store, "b", 10 * MINUTE, &["hosta"], &[("hosta", 130)], "urgent");
    change(&store, "c", 20 * MINUTE, &["hosta", "hostb"], &[("hosta", 30)], "routine");
    let rows = evaluate(&store);
    let current = |id: &str| rows[id]["current"].as_str().unwrap().to_owned();
    // a (55 s) and c (open 20 minutes): the nearest rank of p50 over two is the first.
    assert_eq!(current("ops/change-rollout/p50{class=routine}"), "p50 55 s over 2 samples (1 open over)");
    assert_eq!(current("ops/change-rollout/p50{class=urgent}"), "p50 2.2 min over 1 sample");
    // hosta stopped in 40 s, 130 s and 30 s; hostb in 55 s and is open on c for 20 minutes.
    assert_eq!(current("ops/change-rollout/p95{member=hosta}"), "p95 2.2 min over 3 samples");
    assert_eq!(current("ops/change-rollout/p95{member=hostb}"), "p95 20 min over 2 samples (1 open over)");
    assert!(rows["ops/change-rollout/p95{member=hostb}"]["explain"].as_str().unwrap().contains("custom/acme/change/c on hostb (open 20 min)"));
    assert_eq!(rows.len(), 2 * 5);
}

#[test]
fn a_closed_change_belongs_to_the_window_by_its_stop_time_and_older_ones_are_dropped() {
    let store = store_with(&declaration("", "1h", ""));
    // Ready two hours ago, stopped 30 minutes ago: in the window.
    change(&store, "late", 120 * MINUTE, &["hosta"], &[("hosta", 90 * 60)], "routine");
    // Stopped 100 minutes ago: out of it.
    change(&store, "old", 120 * MINUTE, &["hosta"], &[("hosta", 20 * 60)], "routine");
    let p50 = &evaluate(&store)["ops/change-rollout/p50"];
    assert_eq!(p50["current"], "p50 90 min over 1 sample");
    let evaluations = store.fractal.extensions.get::<Evaluations>();
    let tracked = evaluations.0.lock().unwrap();
    assert_eq!(tracked["objective/ops/change-rollout"].samples.keys().collect::<Vec<_>>(), ["custom/acme/change/late"]);
}

#[test]
fn fewer_samples_than_the_minimum_is_no_data_and_none_at_all_says_so() {
    let source = declaration("", "7d", "").replace("window \"7d\"", "window \"7d\" min-samples=3");
    let store = store_with(&source);
    let empty = &evaluate(&store)["ops/change-rollout/p50"];
    assert_eq!((empty["current"].as_str(), empty["status"].as_str()), (Some("no samples"), Some("no-data")));
    assert!(empty["remaining_percent"].is_null());
    change(&store, "a", 10 * MINUTE, &["hosta"], &[("hosta", 400)], "routine");
    change(&store, "b", 10 * MINUTE, &["hosta"], &[("hosta", 500)], "routine");
    let p50 = &evaluate(&store)["ops/change-rollout/p50"];
    assert_eq!(p50["current"], "p50 6.7 min over 2 samples, needs 3");
    assert_eq!(p50["status"], "no-data", "both over, but too few to judge");
    assert!(p50["remaining_percent"].is_null());
    change(&store, "c", 10 * MINUTE, &["hosta"], &[("hosta", 600)], "routine");
    assert_eq!(evaluate(&store)["ops/change-rollout/p50"]["status"], "missed");
}

#[test]
fn a_new_revision_reads_the_claims_again() {
    let store = store_with(&declaration("", "7d", ""));
    change(&store, "a", 10 * MINUTE, &["hosta"], &[("hosta", 40)], "routine");
    change(&store, "b", 10 * MINUTE, &["hosta"], &[("hosta", 50)], "urgent");
    assert_eq!(evaluate(&store).len(), 2);
    // The first revision kept no `class` field; the label needs the claims read again.
    apply(&store, &declaration("    label \"class\"\n", "7d", ""));
    let rows = evaluate(&store);
    assert_eq!(rows["ops/change-rollout/p50{class=routine}"]["current"], "p50 40 s over 1 sample");
    assert_eq!(rows["ops/change-rollout/p50{class=urgent}"]["current"], "p50 50 s over 1 sample");
    assert_eq!(rows["ops/change-rollout/p50"]["current"], "p50 40 s over 2 samples");
}

#[test]
fn an_evaluation_reads_only_the_claims_after_its_cursor() {
    let store = store_with(&declaration("", "7d", ""));
    change(&store, "a", 10 * MINUTE, &["hosta"], &[("hosta", 40)], "routine");
    evaluate(&store);
    let evaluations = store.fractal.extensions.get::<Evaluations>();
    let cursor = {
        let mut tracked = evaluations.0.lock().unwrap();
        let tracked = tracked.get_mut("objective/ops/change-rollout").unwrap();
        // Forget what was read: a reread would bring `a` back.
        tracked.samples.clear();
        tracked.cursor
    };
    // Claims of other kinds or under another prefix do not move it.
    store
        .append_claim(&ClaimInput {
            subject: "custom/acme/other/z".into(),
            kind: READY.into(),
            actor: None,
            fields: BTreeMap::from([("ready_at".into(), json!(*NOW))]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    claim(&store, "a", "custom.acme.note", json!({}));
    change(&store, "b", 10 * MINUTE, &["hosta"], &[("hosta", 50)], "routine");
    assert_eq!(evaluate(&store)["ops/change-rollout/p50"]["current"], "p50 50 s over 1 sample");
    let tracked = evaluations.0.lock().unwrap();
    let tracked = &tracked["objective/ops/change-rollout"];
    assert!(tracked.cursor > cursor);
    assert_eq!(tracked.samples.keys().collect::<Vec<_>>(), ["custom/acme/change/b"]);
}

#[test]
fn a_claim_without_a_readable_time_or_member_is_counted_unreadable() {
    let store = store_with(&declaration("", "7d", ""));
    change(&store, "a", 10 * MINUTE, &["hosta"], &[("hosta", 40)], "routine");
    claim(&store, "a", APPLIED, json!({"applied_at": "yesterday", "host": "hostb"}));
    claim(&store, "a", APPLIED, json!({"applied_at": *NOW}));
    let explain = evaluate(&store)["ops/change-rollout/p50"]["explain"].as_str().unwrap().to_owned();
    assert!(explain.contains("2 unreadable claims"), "{explain}");
}

fn record(kind: &str, accepted: u64, fields: Value) -> ClaimRecord {
    ClaimRecord {
        id: format!("{kind}-{accepted}"),
        store_index: 0,
        batch_id: String::new(),
        subject: "custom/acme/change/p".into(),
        kind: kind.into(),
        origin: "node".into(),
        actor: None,
        operation_id: None,
        request_digest: None,
        body: json!({ "fields": fields }),
        predecessors: Vec::new(),
        accepted_at_unix_ms: u128::from(accepted),
    }
}

#[test]
fn paused_time_is_not_counted_and_a_paused_change_does_not_age() {
    let source = declaration("    pause \"custom.acme.change-held\" resume=\"custom.acme.change-released\"\n", "7d", "");
    let document = crate::objective::parse_document(&source).unwrap();
    let Indicator::Interval(interval) = &document[0].indicator else { panic!("an interval") };
    let start = *NOW - 30 * MINUTE;
    let mut sample = Sample::default();
    sample.observe(interval, &record(READY, start, json!({"ready_at": start}))).unwrap();
    sample.observe(interval, &record(MERGED, start, json!({"merged_at": start, "hosts": ["hosta"]}))).unwrap();
    sample.observe(interval, &record("custom.acme.change-held", start + 5 * MINUTE, json!({}))).unwrap();
    sample.observe(interval, &record("custom.acme.change-released", start + 15 * MINUTE, json!({}))).unwrap();
    sample.observe(interval, &record("custom.acme.change-held", start + 20 * MINUTE, json!({}))).unwrap();
    // Thirty minutes old, ten paused and still paused for the last ten: it aged ten minutes.
    assert_eq!(sample.class(interval, None, *NOW), Class::Open { age: 10 * MINUTE });
    assert_eq!(sample.class(interval, None, *NOW + 60 * MINUTE), Class::Open { age: 10 * MINUTE });
    sample.observe(interval, &record("custom.acme.change-released", start + 25 * MINUTE, json!({}))).unwrap();
    sample.observe(interval, &record(APPLIED, start + 28 * MINUTE, json!({"host": "hosta", "applied_at": start + 28 * MINUTE}))).unwrap();
    assert_eq!(sample.class(interval, None, *NOW), Class::Closed { value: 13 * MINUTE, stop: start + 28 * MINUTE });
}

const RUN: &str = "custom.acme.run";

/// An interval whose one claim is the whole sample, from its `created_at` to its `completed_at`.
fn run_declaration(only: &str) -> String {
    format!(
        r#"version 2
objective "ops/run-latency" {{
  about "A run, from created to completed"
  indicator "interval" {{
    subject-prefix "custom/acme/change/"
    start "{RUN}" at="created_at"
    stop "{RUN}" at="completed_at"
{only}  }}
  window "1d"
  target "p50" within="5m"
}}
"#
    )
}

/// A run created `ago_ms` before NOW and completed `took_ms` later, with `fields` besides.
fn run(store: &Store, name: &str, ago_ms: u64, took_ms: u64, fields: Value) {
    let mut fields = fields.as_object().unwrap().clone();
    let created = *NOW - ago_ms;
    fields.insert("created_at".into(), json!(created));
    fields.insert("completed_at".into(), json!(created + took_ms));
    claim(store, name, RUN, Value::Object(fields));
}

#[test]
fn one_claim_that_starts_and_stops_the_sample_is_its_stop_less_its_start() {
    let store = store_with(&run_declaration(""));
    run(&store, "a", 30 * MINUTE, 2 * MINUTE, json!({}));
    run(&store, "b", 30 * MINUTE, 7 * MINUTE, json!({}));
    let p50 = &evaluate(&store)["ops/run-latency/p50"];
    assert_eq!(p50["current"], "p50 2 min over 2 samples");
    assert!(p50["explain"].as_str().unwrap().contains("2 closed, 0 open over 5 min, 0 open within it, 0 cancelled"));

    let document = crate::objective::parse_document(&run_declaration("")).unwrap();
    let Indicator::Interval(interval) = &document[0].indicator else { panic!("an interval") };
    let created = *NOW - 10 * MINUTE;
    let mut sample = Sample::default();
    // Its stop cannot be read: the start it could be read for is not taken either.
    assert!(sample.observe(interval, &record(RUN, *NOW, json!({"created_at": created, "completed_at": "soon"}))).is_none());
    assert!(sample.start.is_none() && sample.stops.is_empty());
    sample.observe(interval, &record(RUN, *NOW, json!({"created_at": created, "completed_at": created + 90_000}))).unwrap();
    assert_eq!(sample.class(interval, None, *NOW), Class::Closed { value: 90_000, stop: created + 90_000 });
}

#[test]
fn two_claims_starting_one_sample_at_the_same_time_keep_the_same_one_in_any_order() {
    let document = crate::objective::parse_document(&run_declaration("    only \"attempt\" \"1\"\n")).unwrap();
    let Indicator::Interval(interval) = &document[0].indicator else { panic!("an interval") };
    let created = *NOW - 10 * MINUTE;
    // Two pollers recorded one run at different attempts: the same `created_at`, other ids.
    let attempt = |id: &str, attempt: u64, completed: u64| ClaimRecord {
        id: id.into(),
        ..record(RUN, *NOW, json!({"created_at": created, "completed_at": completed, "attempt": attempt}))
    };
    let (first, second) = (attempt("claim-a", 1, created + 60_000), attempt("claim-b", 2, created + 90_000));
    for order in [[&first, &second], [&second, &first]] {
        let mut sample = Sample::default();
        for claim in order {
            sample.observe(interval, claim).unwrap();
        }
        assert_eq!(sample.class(interval, None, *NOW), Class::Closed { value: 60_000, stop: created + 60_000 });
    }
}

#[test]
fn a_sample_takes_its_stop_from_its_start_claim_not_from_another_claim_that_stopped_earlier() {
    let document = crate::objective::parse_document(&run_declaration("    only \"attempt\" \"1\"\n")).unwrap();
    let Indicator::Interval(interval) = &document[0].indicator else { panic!("an interval") };
    let created = *NOW - 10 * MINUTE;
    let attempt = |id: &str, attempt: u64, created: u64, completed: u64| ClaimRecord {
        id: id.into(),
        ..record(RUN, *NOW, json!({"created_at": created, "completed_at": completed, "attempt": attempt}))
    };
    // The start claim (earliest by time, then id) completes after one `only` leaves out, which
    // starts later or ties and stops first: its earlier stop must not close the start claim's sample.
    let start = attempt("claim-b", 1, created, created + 90_000);
    for other in [attempt("claim-c", 2, created + 10_000, created + 30_000), attempt("claim-c", 2, created, created + 30_000)] {
        for order in [[&start, &other], [&other, &start]] {
            let mut sample = Sample::default();
            for claim in order {
                sample.observe(interval, claim).unwrap();
            }
            assert_eq!(sample.class(interval, None, *NOW), Class::Closed { value: 90_000, stop: created + 90_000 });
        }
    }
}

#[test]
fn only_leaves_out_a_sample_whose_start_claim_has_another_value_or_none() {
    let store = store_with(&run_declaration("    only \"event\" \"merge_group\"\n    only \"attempt\" \"1\"\n"));
    run(&store, "kept", 30 * MINUTE, 2 * MINUTE, json!({"event": "merge_group", "attempt": 1}));
    run(&store, "pull", 30 * MINUTE, 4 * MINUTE, json!({"event": "pull_request", "attempt": 1}));
    run(&store, "retry", 30 * MINUTE, 3 * MINUTE, json!({"event": "merge_group", "attempt": 2}));
    run(&store, "unnamed", 30 * MINUTE, 6 * MINUTE, json!({"attempt": 1}));
    let p50 = &evaluate(&store)["ops/run-latency/p50"];
    assert_eq!(p50["current"], "p50 2 min over 1 sample");
    let explain = p50["explain"].as_str().unwrap();
    assert!(explain.contains("1 closed, 0 open over 5 min, 0 open within it, 0 cancelled, 3 excluded or left out, 0 unreadable claims."), "{explain}");
}

#[test]
fn a_declaration_this_build_cannot_read_is_a_row_that_says_so() {
    // As a replicated body from a newer build reads here.
    let error = crate::objective::parse(&json!({"name": "objective", "arguments": ["ops/next"]})).unwrap_err();
    let declaration = DeclaredObjective {
        subject: "objective/ops/next".into(),
        revision: "0123456789abcdef".into(),
        claim_id: "claim-1".into(),
        objective: Err(format!("{}: {}", error.code, error.message)),
    };
    let row = Tracked::new(&declaration).unreadable_row(&declaration.subject, declaration.name());
    assert_eq!((row["id"].as_str(), row["status"].as_str()), (Some("ops/next"), Some("no-data")));
    assert_eq!(row["current"], "unreadable declaration");
    let explain = row["explain"].as_str().unwrap();
    assert!(explain.starts_with("objective/ops/next at revision 0123456789ab (claim claim-1) does not decode on this build: invalid-objective"), "{explain}");
}

fn with_owner(store: &Store, host: &str) {
    let source = format!("version 2\nagent \"{host}.ops\" {{ command \"true\"; host \"{host}\" }}\n");
    apply(store, &source);
    let owner = format!("agent/{host}.ops");
    assert_eq!(store.agent_host(&owner).unwrap().as_deref(), Some(host));
    assert!(store.fault_reviewer_is_declared(&owner).unwrap());
}

const BREACH: &str = "  on-breach \"fault\" owner=\"agent/node.ops\" backup=\"agent/node.backup\" condition=\"change-rollout-slo-missed\"\n";

#[test]
fn a_missed_row_is_a_fault_for_its_owner_on_the_member_the_owner_is_declared_on() {
    let store = store_with(&declaration("", "7d", BREACH));
    with_owner(&store, "node");
    change(&store, "a", 30 * MINUTE, &["hosta"], &[("hosta", 600)], "routine");
    let rows = rows(&store, *NOW).unwrap();
    let explain = rows[0]["explain"].as_str().unwrap();
    assert!(explain.contains("addressed to agent/node.ops, delivered by node, the member agent/node.ops is declared on (this member). agent/node.backup is the held backup"), "{explain}");
    let opened = check(&store, &rows).unwrap();
    assert_eq!(opened.opened, 2, "p50 and p95 are both missed");
    let failure = store.operational_failure("slo-objective/node/ops/change-rollout/p95").unwrap().unwrap();
    assert_eq!((failure.status.as_str(), failure.reviewer.as_str()), ("pending", "agent/node.ops"), "the owner, never the backup");
    assert_eq!(check(&store, &rows).unwrap(), ObjectiveFaults::default(), "one episode while missed");

    // Twenty fast changes put the slow one within both allowances: met again, the fault ends.
    for name in ["b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u"] {
        change(&store, name, 10 * MINUTE, &["hosta"], &[("hosta", 10)], "routine");
    }
    let rows = super::rows(&store, *NOW).unwrap();
    assert_eq!(rows.iter().map(|row| row["status"].as_str().unwrap()).collect::<Vec<_>>(), ["met", "met"]);
    assert_eq!(check(&store, &rows).unwrap().recovered, 2);
    assert_eq!(store.operational_failure("slo-objective/node/ops/change-rollout/p95").unwrap().unwrap().status, "resolved");
}

#[test]
fn an_owner_declared_on_another_member_is_delivered_there_not_here() {
    let store = store_with(&declaration("", "7d", &BREACH.replace("node.ops", "other.ops")));
    with_owner(&store, "other");
    change(&store, "a", 30 * MINUTE, &["hosta"], &[("hosta", 600)], "routine");
    let rows = rows(&store, *NOW).unwrap();
    assert_eq!(rows[0]["status"], "missed");
    assert!(rows[0]["explain"].as_str().unwrap().contains("delivered by other, the member agent/other.ops is declared on."));
    assert_eq!(check(&store, &rows).unwrap(), ObjectiveFaults::default());
    assert!(store.operational_failure("slo-objective/node/ops/change-rollout/p95").unwrap().is_none());
}

#[test]
fn an_owner_that_is_not_declared_gets_nothing_and_the_explanation_says_so() {
    let store = store_with(&declaration("", "7d", BREACH));
    change(&store, "a", 30 * MINUTE, &["hosta"], &[("hosta", 600)], "routine");
    let rows = rows(&store, *NOW).unwrap();
    assert!(rows[0]["explain"].as_str().unwrap().contains("agent/node.ops is not a live agent declaration, so nothing is delivered"));
    assert_eq!(check(&store, &rows).unwrap(), ObjectiveFaults::default());
}

#[test]
fn spans_and_field_values_read_the_ways_claims_write_them() {
    assert_eq!([span(7 * 86_400_000), span(90 * MINUTE), span(1_500)], ["7d", "90m", "1500ms"]);
    assert_eq!(time_ms(&json!("2026-11-09T10:00:00Z")), Some(1_794_218_400_000));
    assert_eq!(time_ms(&json!("1794218400000")), Some(1_794_218_400_000));
    assert_eq!(time_ms(&json!("\"2026-11-09T10:00:00Z\"")), Some(1_794_218_400_000));
    assert_eq!(time_ms(&json!("soon")), None);
    assert_eq!(string_list(&json!("[\"a\",\"b\"]")), Some(vec![s("a"), s("b")]));
    assert_eq!(string_list(&json!(["a", 1])), None);
    assert_eq!(text(&json!("\"routine\"")).as_deref(), Some("routine"));
}

fn count_declaration(extra: &str, window: &str, target: &str, breach: &str) -> String {
    format!(
        r#"version 2
objective "factory/work-throughput" {{
  about "Pull requests merged"
  indicator "count" {{
    subject-prefix "custom/acme/change/"
    event "{MERGED}" at="merged_at"
{extra}  }}
  window "{window}"
  target "per-hour" {target}
{breach}}}
"#
    )
}

/// A merge of `change` `ago_ms` before NOW, with `fields` besides its time.
fn merged(store: &Store, change: &str, ago_ms: u64, fields: Value) {
    let mut fields = fields.as_object().unwrap().clone();
    fields.insert("merged_at".into(), json!(*NOW - ago_ms));
    claim(store, change, MERGED, Value::Object(fields));
}

const THROUGHPUT: &str = "factory/work-throughput";

#[test]
fn a_count_is_no_data_until_its_first_event_and_an_empty_window_is_then_a_real_zero() {
    let store = store_with(&count_declaration("", "1h", "min=1", ""));
    let empty = &evaluate(&store)[THROUGHPUT];
    assert_eq!((empty["status"].as_str(), empty["current"].as_str()), (Some("no-data"), Some("no events yet")));
    assert!(empty["remaining_percent"].is_null());
    assert_eq!(empty["target"], "≥ 1 an hour");
    claim(&store, "a", MERGED, json!({"merged_at": "yesterday"}));
    let unreadable = &evaluate(&store)[THROUGHPUT];
    assert_eq!(unreadable["status"], "no-data", "an unreadable claim changes nothing");
    assert!(unreadable["explain"].as_str().unwrap().contains("0 counted, 0 excluded or left out, 1 unreadable claim."));
    // Read, then before the window: the producer ran and has stopped.
    merged(&store, "b", 2 * 60 * MINUTE, json!({}));
    let stopped = &evaluate(&store)[THROUGHPUT];
    assert_eq!(stopped["current"], "0 in the last 1h (0.00 an hour)");
    assert_eq!((stopped["status"].as_str(), stopped["remaining_percent"].as_f64()), (Some("missed"), Some(0.0)));
}

/// The subjects a count holds in memory.
fn held(store: &Store) -> Vec<String> {
    let evaluations = store.fractal.extensions.get::<Evaluations>();
    let tracked = evaluations.0.lock().unwrap();
    tracked.values().flat_map(|tracked| tracked.events.keys().cloned()).collect()
}

#[test]
fn a_subject_counts_once_at_its_earliest_event_whoever_writes_it() {
    let store = store_with(&count_declaration("", "1h", "max=10", ""));
    // A producer, its retry and a second producer all carry the merge's time.
    for _ in 0..3 {
        merged(&store, "a", 10 * MINUTE, json!({}));
    }
    // A claim with a later time does not move it.
    merged(&store, "a", 5 * MINUTE, json!({}));
    // Merged before the window: read, not counted, not held.
    merged(&store, "b", 2 * 60 * MINUTE, json!({}));
    let row = &evaluate(&store)[THROUGHPUT];
    assert_eq!(row["current"], "1 in the last 1h (1.00 an hour)");
    assert!(row["explain"].as_str().unwrap().contains("Each subject under custom/acme/change/ counts once, at its earliest custom.acme.change-merged claim."));
    assert_eq!(held(&store), [s("custom/acme/change/a")]);
    // A retry inside the window is deduplicated; one of b after the window carries its old time.
    merged(&store, "a", 10 * MINUTE, json!({}));
    merged(&store, "b", 2 * 60 * MINUTE, json!({}));
    assert_eq!(evaluate(&store)[THROUGHPUT]["current"], "1 in the last 1h (1.00 an hour)");
    assert_eq!(held(&store), [s("custom/acme/change/a")]);
}

#[test]
fn an_event_leaves_the_count_and_memory_when_it_leaves_the_window() {
    let store = store_with(&count_declaration("", "1h", "max=10", ""));
    merged(&store, "a", 50 * MINUTE, json!({}));
    merged(&store, "b", 10 * MINUTE, json!({}));
    assert_eq!(evaluate(&store)[THROUGHPUT]["current"], "2 in the last 1h (2.00 an hour)");
    let later = rows(&store, *NOW + 20 * MINUTE).unwrap();
    assert_eq!(later[0]["current"], "1 in the last 1h (1.00 an hour)");
    assert_eq!(held(&store), [s("custom/acme/change/b")], "memory holds only the subjects in the window");
    // A retry of the pruned subject carries its time, before the window: ignored.
    merged(&store, "a", 50 * MINUTE, json!({}));
    assert_eq!(rows(&store, *NOW + 20 * MINUTE).unwrap()[0]["current"], "1 in the last 1h (1.00 an hour)");
    assert_eq!(held(&store), [s("custom/acme/change/b")]);
}

#[test]
fn a_minimum_and_a_maximum_rate_are_met_or_missed_with_their_budget() {
    for (target, status, remaining) in [
        ("min=2", "met", 33.3),
        ("min=4", "missed", 0.0),
        ("max=4", "met", 25.0),
        ("max=2", "missed", 0.0),
        ("max=0", "missed", 0.0),
    ] {
        let store = store_with(&count_declaration("", "2h", target, ""));
        for change in ["a", "b", "c", "d", "e", "f"] {
            merged(&store, change, 30 * MINUTE, json!({}));
        }
        let row = &evaluate(&store)[THROUGHPUT];
        assert_eq!(row["current"], "6 in the last 2h (3.00 an hour)", "{target}");
        assert_eq!((row["status"].as_str(), row["remaining_percent"].as_f64()), (Some(status), Some(remaining)), "{target}");
    }
    let store = store_with(&count_declaration("", "1h", "max=0", ""));
    merged(&store, "old", 2 * 60 * MINUTE, json!({}));
    let row = &evaluate(&store)[THROUGHPUT];
    assert_eq!((row["status"].as_str(), row["remaining_percent"].as_f64()), (Some("met"), Some(100.0)));
    assert_eq!(row["target"], "≤ 0 an hour");
}

#[test]
fn labels_split_a_count_and_an_excluded_event_is_counted_apart() {
    let store = store_with(&count_declaration("    label \"repository\"\n    exclude \"author\" \"bot\"\n", "1h", "min=1", ""));
    merged(&store, "a", 10 * MINUTE, json!({"repository": "fractal", "author": "agent"}));
    merged(&store, "b", 10 * MINUTE, json!({"repository": "fractal", "author": "bot"}));
    merged(&store, "c", 10 * MINUTE, json!({"repository": "factory"}));
    let rows = evaluate(&store);
    assert_eq!(
        rows.keys().cloned().collect::<Vec<_>>(),
        [s(THROUGHPUT), s("factory/work-throughput{repository=factory}"), s("factory/work-throughput{repository=fractal}")]
    );
    assert_eq!(rows[THROUGHPUT]["current"], "2 in the last 1h (2.00 an hour)");
    let fractal = &rows["factory/work-throughput{repository=fractal}"];
    assert_eq!(fractal["current"], "1 in the last 1h (1.00 an hour)");
    assert!(fractal["explain"].as_str().unwrap().contains("1 counted, 1 excluded or left out, 0 unreadable claims."));
}

#[test]
fn a_new_count_revision_reads_the_claims_again() {
    let store = store_with(&count_declaration("", "1h", "min=1", ""));
    merged(&store, "a", 10 * MINUTE, json!({"repository": "fractal"}));
    assert_eq!(evaluate(&store).len(), 1);
    apply(&store, &count_declaration("    label \"repository\"\n", "1h", "min=1", ""));
    let rows = evaluate(&store);
    assert_eq!(rows["factory/work-throughput{repository=fractal}"]["current"], "1 in the last 1h (1.00 an hour)");
}

#[test]
fn a_missed_count_is_a_fault_for_its_owner() {
    let store = store_with(&count_declaration("", "1h", "min=2", BREACH));
    with_owner(&store, "node");
    merged(&store, "a", 10 * MINUTE, json!({}));
    let rows = rows(&store, *NOW).unwrap();
    assert_eq!(rows[0]["status"], "missed");
    assert_eq!(check(&store, &rows).unwrap().opened, 1);
    let failure = store.operational_failure("slo-objective/node/factory/work-throughput").unwrap().unwrap();
    assert_eq!(failure.reviewer, "agent/node.ops");
}

#[test]
fn a_count_label_row_is_a_part_shown_without_a_verdict_while_the_total_is_judged() {
    // At least 2 an hour in all: 3 is met, though no repository alone reaches 2.
    let store = store_with(&count_declaration("    label \"repository\"\n", "1h", "min=2", ""));
    merged(&store, "a", 10 * MINUTE, json!({"repository": "fractal"}));
    merged(&store, "b", 20 * MINUTE, json!({"repository": "fractal"}));
    merged(&store, "c", 30 * MINUTE, json!({"repository": "factory"}));
    let rows = evaluate(&store);
    let total = &rows[THROUGHPUT];
    assert_eq!((total["status"].as_str(), total["target"].as_str()), (Some("met"), Some("≥ 2 an hour")));
    assert_eq!(total["current"], "3 in the last 1h (3.00 an hour)");
    let factory = &rows["factory/work-throughput{repository=factory}"];
    assert_eq!(factory["current"], "1 in the last 1h (1.00 an hour)");
    assert_eq!(
        (factory["status"].as_str(), factory["target"].as_str()),
        (Some("no-data"), Some("part of factory/work-throughput")),
        "a part is never met or missed against the whole count's minimum"
    );
    assert!(factory["remaining_percent"].is_null());
    let explain = factory["explain"].as_str().unwrap();
    assert!(explain.contains("those whose repository is factory"), "{explain}");
    assert!(explain.contains("It applies to factory/work-throughput as a whole: this part of it is shown, never met or missed."), "{explain}");
}

#[test]
fn a_missed_count_opens_one_fault_for_its_total_and_none_for_its_parts() {
    let store = store_with(&count_declaration("    label \"repository\"\n", "1h", "min=4", BREACH));
    with_owner(&store, "node");
    merged(&store, "a", 10 * MINUTE, json!({"repository": "fractal"}));
    merged(&store, "c", 10 * MINUTE, json!({"repository": "factory"}));
    let rows = rows(&store, *NOW).unwrap();
    assert_eq!(rows.iter().map(|row| row["status"].as_str().unwrap()).collect::<Vec<_>>(), ["missed", "no-data", "no-data"]);
    assert_eq!(check(&store, &rows).unwrap().opened, 1);
    assert!(store.operational_failure("slo-objective/node/factory/work-throughput").unwrap().is_some());
    assert!(store.operational_failure("slo-objective/node/factory/work-throughput{repository=fractal}").unwrap().is_none());
}

#[test]
fn only_and_exclude_leave_events_out_of_a_count_and_both_are_counted_apart() {
    let store = store_with(&count_declaration("    only \"workflow\" \"Workspace CI\"\n    exclude \"flakes\" \"0\"\n", "1h", "max=0", ""));
    merged(&store, "flaky", 10 * MINUTE, json!({"workflow": "Workspace CI", "flakes": 2}));
    merged(&store, "clean", 10 * MINUTE, json!({"workflow": "Workspace CI", "flakes": 0}));
    merged(&store, "other", 10 * MINUTE, json!({"workflow": "Release", "flakes": 3}));
    merged(&store, "unnamed", 10 * MINUTE, json!({"flakes": 1}));
    let row = &evaluate(&store)[THROUGHPUT];
    assert_eq!(row["current"], "1 in the last 1h (1.00 an hour)");
    assert_eq!(row["status"], "missed");
    let explain = row["explain"].as_str().unwrap();
    assert!(explain.contains("1 counted, 3 excluded or left out, 0 unreadable claims."), "{explain}");
}

#[test]
fn a_count_label_row_reads_zero_once_its_events_leave_the_window() {
    let store = store_with(&count_declaration("    label \"repository\"\n", "1h", "min=1", ""));
    merged(&store, "a", 50 * MINUTE, json!({"repository": "fractal"}));
    merged(&store, "b", 10 * MINUTE, json!({"repository": "factory"}));
    // Merged before the window: its repository is read, so its row is there from the start.
    merged(&store, "c", 2 * 60 * MINUTE, json!({"repository": "smallclaims"}));
    let ids = |rows: &[Value]| rows.iter().map(|row| row["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    let expected = [
        s(THROUGHPUT),
        s("factory/work-throughput{repository=factory}"),
        s("factory/work-throughput{repository=fractal}"),
        s("factory/work-throughput{repository=smallclaims}"),
    ];
    let now = rows(&store, *NOW).unwrap();
    assert_eq!(ids(&now[..]), expected);
    assert_eq!(now[3]["current"], "0 in the last 1h (0.00 an hour)");
    let later = rows(&store, *NOW + 20 * MINUTE).unwrap();
    assert_eq!(ids(&later[..]), expected, "a label value read once keeps its row");
    assert_eq!(later[2]["current"], "0 in the last 1h (0.00 an hour)");
    assert_eq!(held(&store), [s("custom/acme/change/b")], "only the events in the window are held");
    // A new revision reads the claims again, keeping the values its own label reads.
    apply(&store, &count_declaration("    label \"author\"\n", "1h", "min=1", ""));
    assert_eq!(ids(&rows(&store, *NOW + 20 * MINUTE).unwrap()[..]), [s(THROUGHPUT)]);
}

/// An open fault for objective row `id`, as an earlier evaluation, or build, opened it.
fn open_fault(store: &Store, id: &str) {
    store
        .append_claim(&ClaimInput {
            subject: s("daemon/node"),
            kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("episode".into(), json!(crate::objective_faults::episode("node", id))),
                ("condition".into(), json!("change-rollout-slo-missed")),
                ("reviewer".into(), json!("agent/node.ops")),
                ("title".into(), json!(format!("{id} missed"))),
                ("reason".into(), json!("An earlier evaluation missed it.")),
                ("severity".into(), json!("warning")),
                ("targets".into(), json!(["daemon/node"])),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

fn fault_status(store: &Store, id: &str) -> String {
    store.operational_failure(&crate::objective_faults::episode("node", id)).unwrap().unwrap().status
}

#[test]
fn the_open_fault_of_a_count_label_row_ends_and_another_objectives_stays() {
    let store = store_with(&count_declaration("    label \"repository\"\n", "1h", "min=1", BREACH));
    with_owner(&store, "node");
    let part = "factory/work-throughput{repository=fractal}";
    let deeper = "factory/work-throughput/flakes";
    for id in [part, deeper, "flakes"] {
        open_fault(&store, id);
    }
    merged(&store, "a", 10 * MINUTE, json!({"repository": "fractal"}));
    let rows = rows(&store, *NOW).unwrap();
    assert_eq!(rows[0]["status"], "met");
    assert_eq!(check(&store, &rows).unwrap(), ObjectiveFaults { recovered: 1, ..ObjectiveFaults::default() });
    assert_eq!(fault_status(&store, part), "resolved", "a part is never judged, so its fault ends");
    assert_eq!((fault_status(&store, deeper), fault_status(&store, "flakes")), (s("pending"), s("pending")));
}

#[test]
fn the_fault_of_an_interval_label_row_ends_once_its_samples_leave_the_window() {
    let store = store_with(&declaration("    label \"class\"\n", "7d", BREACH));
    with_owner(&store, "node");
    change(&store, "a", 30 * MINUTE, &["hosta"], &[("hosta", 600)], "routine");
    let missed = rows(&store, *NOW).unwrap();
    assert_eq!(check(&store, &missed).unwrap().opened, 4, "p50 and p95, in all and for class routine");
    // Eight days on, the sample has left the window: the class rows are gone and the totals are
    // no-data, which neither opens nor ends a fault.
    let later = rows(&store, *NOW + 8 * 24 * 60 * MINUTE).unwrap();
    assert_eq!(later.iter().map(|row| row["status"].as_str().unwrap()).collect::<Vec<_>>(), ["no-data", "no-data"]);
    assert_eq!(check(&store, &later).unwrap(), ObjectiveFaults { recovered: 2, ..ObjectiveFaults::default() });
    assert_eq!(fault_status(&store, "ops/change-rollout/p95{class=routine}"), "resolved");
    assert_eq!(fault_status(&store, "ops/change-rollout/p95"), "pending");
}

#[test]
fn an_objective_owns_its_rows_and_not_a_deeper_objectives() {
    for id in ["a/b", "a/b/p95", "a/b{x=y}", "a/b/p95{x=y/p1}"] {
        assert!(owns("a/b", id), "{id}");
    }
    for id in ["a/bc", "a/b/c", "a/b/p", "a/b/pq", "a/b/p95x", "a/b/c{x=y}", "a"] {
        assert!(!owns("a/b", id), "{id}");
    }
}
