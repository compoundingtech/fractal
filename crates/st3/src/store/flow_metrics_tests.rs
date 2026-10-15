use super::*;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

fn fixture() -> Store {
    let store = Store::open_memory("example").unwrap();
    store.connection.write().execute_batch(
        "INSERT INTO mission_runs VALUES('run','example','rev','gen','rev','run',NULL,'/tmp','person/example','{}','run','running','normal','1','1');
         INSERT INTO run_generations VALUES('gen','run','rev',NULL,'running','person/example','test','1','1');"
    ).unwrap();
    store
}

fn step(store: &Store, id: &str, state: &str, person: Option<&str>, holder: Option<&str>, expiry: Option<i64>, created: i64) {
    store.connection.write().execute(
        "INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,status,attempt,assignee,goals,lease_owner,lease_incarnation,lease_expires_at_unix_ms,created_at_unix_ms,updated_at_unix_ms)
         VALUES(?1,'run','gen',?1,'hash',?2,1,?3,'[]',?4,'inc',?5,?6,?6)",
        params![id, state, person, holder, expiry.map(|n| n.to_string()), created.to_string()],
    ).unwrap();
}

#[test]
fn flow_metrics_states_expiry_deduplication_and_idle_threshold() {
    let store = fixture();
    let now = 2_000_000;
    step(&store, "ready", "ready", None, None, None, 1000);
    step(&store, "claim", "claimed", None, Some("agent/example/idle"), Some(now+1), 1000);
    step(&store, "working", "working", None, Some("agent/example/idle"), Some(now+1), 1000);
    step(&store, "verify", "verifying", None, Some("agent/example/recent"), Some(now+1), now-600_000);
    step(&store, "expired", "claimed", None, Some("agent/example/expired"), Some(now), 1000);
    step(&store, "person", "ready", Some("person/example"), None, None, 1000);
    step(&store, "ask", "waiting-person", None, None, None, 1000);
    step(&store, "pending", "pending", None, None, None, 1000);
    step(&store, "blocked", "blocked", None, Some("agent/example/idle"), Some(now+1), 1000);
    step(&store, "done", "completed", None, None, None, 0);
    let value = store.flow_snapshot(now as u128).unwrap();
    assert_eq!(value.counts, [2,3,2,2]);
    assert_eq!(value.oldest_seconds, [1999.0;4]);
    assert_eq!((value.with_work,value.idle_with_work), (2,1));
    store.connection.write().execute("UPDATE mission_runs SET phase='terminal'", []).unwrap();
    let value = store.flow_snapshot(now as u128).unwrap();
    assert_eq!(value.counts, [0;4]);
    assert_eq!(value.with_work, 0);
}

#[test]
fn flow_metrics_export_values_units_and_bounded_labels() {
    let store = Arc::new(fixture());
    let now = i64::try_from(now_ms()).unwrap();
    step(&store, "free text that must never become a label", "ready", None, None, None, now-100_000);
    step(&store, "holder", "working", None, Some("agent/example/private"), Some(now+1_000_000), now-1_000_000);
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder().with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
    crate::otel::register_work_metrics_on(provider.meter("st3"), store.clone());
    provider.force_flush().unwrap();
    let finished = exporter.get_finished_metrics().unwrap();
    let mut names = BTreeSet::new();
    for metric in finished.iter().flat_map(|r| r.scope_metrics()).flat_map(|s| s.metrics()) {
        names.insert(metric.name().to_owned());
        match metric.data() {
            AggregatedMetrics::U64(MetricData::Gauge(gauge)) => {
                let mut sum = 0;
                for point in gauge.data_points() {
                    sum += point.value();
                    for label in point.attributes() {
                        assert_eq!(label.key.as_str(), "st.work.state");
                        assert!(WORK_STATES.contains(&label.value.as_str().as_ref()));
                    }
                    if metric.name().starts_with("st.agents.") { assert_eq!(point.attributes().count(), 0); }
                }
                assert_eq!(sum, if metric.name()=="st.work.items" {2} else {1});
            }
            AggregatedMetrics::F64(MetricData::Gauge(gauge)) => {
                assert_eq!(metric.name(), "st.work.oldest_age");
                assert_eq!(metric.unit(), "s");
                assert_eq!(gauge.data_points().count(),4);
                for point in gauge.data_points() {
                    assert!(point.value() >= 0.0);
                    if point.attributes().any(|label| label.value.as_str() == "ready") {
                        assert!((100.0..101.0).contains(&point.value()));
                    }
                    if point.attributes().any(|label| label.value.as_str() == "claimed") {
                        assert!((1000.0..1001.0).contains(&point.value()));
                    }
                    for label in point.attributes() {
                        assert_eq!(label.key.as_str(), "st.work.state");
                        assert!(WORK_STATES.contains(&label.value.as_str().as_ref()));
                    }
                }
            }
            other => panic!("unexpected metric: {other:?}"),
        }
    }
    assert_eq!(names, BTreeSet::from(["st.work.items".into(),"st.work.oldest_age".into(),"st.agents.with_work".into(),"st.agents.idle_with_work".into()]));
    // The same sample backs every instrument and another export inside 60 seconds.
    store.connection.write().execute("UPDATE step_runs SET status='completed'", []).unwrap();
    provider.force_flush().unwrap();
    let finished = exporter.get_finished_metrics().unwrap();
    let items = finished.last().unwrap().scope_metrics().flat_map(|scope| scope.metrics())
        .find(|metric| metric.name()=="st.work.items").unwrap();
    let AggregatedMetrics::U64(MetricData::Gauge(items)) = items.data() else { panic!("items gauge") };
    assert_eq!(items.data_points().map(|point| point.value()).sum::<u64>(),2);
    provider.shutdown().unwrap();
}

#[test]
fn flow_metrics_cost_is_independent_of_completed_history() {
    use smallclaims::sqlite::work::SqliteWorkScope;
    let measure = |history| {
        let store = fixture();
        step(&store, "current", "claimed", None, Some("agent/example/holder"), Some(9_000_000), 1000);
        for i in 0..history { step(&store, &format!("history-{i}"), "completed", None, None, None, 1); }
        let scope = SqliteWorkScope::start();
        let started = std::time::Instant::now();
        assert_eq!(store.flow_snapshot(2_000_000).unwrap().with_work, 1);
        let elapsed = started.elapsed();
        let cost = scope.finish();
        eprintln!("flow_metrics history={history} elapsed_us={} cost={cost:?}", elapsed.as_micros());
        cost
    };
    let small = measure(10);
    let large = measure(1000);
    assert!(small.vm_steps > 0);
    assert!(large.vm_steps <= small.vm_steps+100, "small={small:?} large={large:?}");
}

#[test]
fn flow_metrics_substantive_activity_excludes_quiet_and_other_incarnations() {
    for (kind, fields, active) in [
        ("work.progress", json!({"summary":"progress"}), true),
        ("work.submitted", json!({"status":"completed","summary":"done"}), true),
        ("message.sent", json!({"from":"agent/example/holder","to":"agent/example/other"}), true),
        ("message.sent", json!({"from":"agent/example/other","to":"agent/example/holder"}), true),
        ("harness.timeline", json!({"incarnation_id":"inc","entry_type":"tool_call"}), true),
        ("harness.timeline", json!({"incarnation_id":"old","entry_type":"tool_call"}), false),
        ("harness.timeline", json!({"incarnation_id":"inc","entry_type":"usage"}), false),
        ("harness.usage", json!({"incarnation_id":"inc"}), false),
        ("work.renewed", json!({}), false),
        ("work.claimed", json!({}), true),
    ] {
        let store = fixture();
        step(&store, "step-run/example/step", "claimed", None, Some("agent/example/holder"), Some(9_000_000), 1000);
        store.set_write_clock_at(1_500_000).unwrap();
        {
            let mut connection = store.connection.write();
            let tx = connection.transaction().unwrap();
            let subject = if kind.starts_with("work.") { "step-run/example/step" } else { "agent/example/holder" };
            append_claim_tx(&tx, &store.origin, subject, kind, Some("agent/example/holder"),
                &json!({"fields":fields}), &[], None).unwrap();
            tx.commit().unwrap();
        }
        let value = store.flow_snapshot(2_000_000).unwrap();
        assert_eq!(value.with_work,1);
        assert_eq!(value.idle_with_work, u64::from(!active), "{kind} {fields}");
    }
}
