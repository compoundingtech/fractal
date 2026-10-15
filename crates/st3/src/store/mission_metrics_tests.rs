use super::*;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

fn seed(store: &Store, first: usize, count: usize, origin: &str, phase: Option<&str>) {
    store.set_write_clock_at(1_000_001).unwrap();
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    for index in first..first+count {
        let id = format!("sample-{index}");
        let status = ["completed","failed","cancelled"][index%3];
        tx.execute(
            "INSERT OR IGNORE INTO mission_runs VALUES(?1,'example','rev','gen','rev',?1,NULL,'/tmp','person/example','{}','run',?2,'terminal','1','1000001')",
            params![id,status],
        ).unwrap();
        let mut fields = json!({"status":status});
        if let Some(phase) = phase { fields["phase"] = json!(phase); }
        append_claim_tx(&tx,origin,&format!("mission-run/{id}"),"mission-run.state",None,
            &json!({"fields":fields}),&[],None).unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn mission_metrics_completion_catchup_preserves_more_than_one_batch_exactly_once() {
    let store = Store::open_memory("example").unwrap();
    seed(&store,9000,1,"example",Some("terminal"));
    let (initial, samples) = store.mission_completion_batch(None).unwrap();
    assert!(samples.is_empty(),"no history backfill on startup");
    seed(&store,0,1200,"example",Some("terminal"));
    let (first, mut samples) = store.mission_completion_batch(Some(initial)).unwrap();
    assert_eq!(samples.len(),COMPLETION_BATCH);
    let (second, tail) = store.mission_completion_batch(Some(first)).unwrap();
    assert_eq!(tail.len(),1200-COMPLETION_BATCH);
    samples.extend(tail);
    let (empty, tail) = store.mission_completion_batch(Some(second)).unwrap();
    assert_eq!(empty,second);
    assert!(tail.is_empty(),"a repeated tick cannot double count");
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder().with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
    let histogram = crate::otel::mission_lead_time_histogram(provider.meter("st3"));
    for sample in &samples {
        assert_eq!(sample.seconds,1000.0);
        crate::otel::record_mission_completion(&histogram,sample);
    }
    provider.force_flush().unwrap();
    let finished = exporter.get_finished_metrics().unwrap();
    let metric = finished.iter().flat_map(|r|r.scope_metrics()).flat_map(|s|s.metrics()).next().unwrap();
    assert_eq!(metric.name(),crate::otel::MISSION_LEAD_TIME);
    assert_eq!(metric.unit(),"s");
    let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() else { panic!("lead time histogram") };
    assert_eq!(histogram.data_points().count(),3);
    for point in histogram.data_points() {
        assert_eq!(point.count(),400);
        assert_eq!(point.sum(),400_000.0);
        assert_eq!(point.bounds().map(|bound| bound.to_owned()).collect::<Vec<f64>>(),
            crate::otel::MISSION_LEAD_TIME_BUCKETS);
        let labels = point.attributes().collect::<Vec<_>>();
        assert_eq!(labels.len(),1);
        assert_eq!(labels[0].key.as_str(),"st.mission.run.outcome");
        assert!(["succeeded","failed","cancelled"].contains(&labels[0].value.as_str().as_ref()));
    }
    provider.shutdown().unwrap();
}

#[test]
fn mission_metrics_completion_skips_replica_and_outcome_corrections() {
    let store = Store::open_memory("example").unwrap();
    let (cursor,_) = store.mission_completion_batch(None).unwrap();
    seed(&store,0,1,"other",Some("terminal"));
    seed(&store,1,1,"example",Some("cleanup-failed"));
    seed(&store,2,1,"example",None);
    seed(&store,3,1,"example",Some("terminal"));
    let (next,samples) = store.mission_completion_batch(Some(cursor)).unwrap();
    assert!(next>cursor);
    assert_eq!(samples.len(),1);
    assert_eq!(samples[0].outcome,MissionOutcome::Succeeded);
}

fn waiting_store() -> (Store, i64) {
    let store = Store::open_memory("example").unwrap();
    let now = i64::try_from(now_ms()).unwrap();
    let mut connection = store.connection.write();
    let tx = connection.transaction().unwrap();
    tx.execute_batch(
        "INSERT INTO mission_runs VALUES('run','example','rev','gen','rev','run',NULL,'/tmp','person/example','{}','run','running','normal','1','1');
         INSERT INTO run_generations VALUES('gen','run','rev',NULL,'running','person/example','test','1','1');"
    ).unwrap();
    for (id, state, assignee, reason) in [
        ("person","ready",Some("person/example"),None),
        ("ask","waiting-person",None,None),
        ("dependency","pending",None,None),
        ("baseline","blocked",None,Some("step baseline `proof` does not hold")),
        ("infrastructure","blocked",None,Some("no eligible agent is present in the desired graph")),
        ("orphan","orphaned",None,None),
        ("declared","working",None,None),
    ] {
        tx.execute(
            "INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,status,attempt,assignee,agentless,goals,lease_owner,lease_incarnation,lease_expires_at_unix_ms,blocked_reason,created_at_unix_ms,updated_at_unix_ms)
             VALUES(?1,'run','gen',?1,'hash',?2,1,?3,0,'[]',NULL,NULL,NULL,?4,?5,?5)",
            params![id,state,assignee,reason,(now-1_800_000).to_string()],
        ).unwrap();
    }
    tx.commit().unwrap();
    // Seed a clean projection at the same interface used by the read-only sampler.
    connection.execute(
        "INSERT OR REPLACE INTO local_step_clocks(subject,attempt,frontier,accepted_at,state)
         VALUES('declared',1,X'00',?1,?2)",
        params![now.to_string(),json!({"started":null,"elapsed":0,"lease_expires":null,
            "paused_until":now+600_000,"pause_started":now-1_200_000,"waited":0}).to_string()],
    ).unwrap();
    connection.execute("DELETE FROM local_step_clock_pending WHERE subject='declared'",[]).unwrap();
    connection.execute("DELETE FROM local_step_clock_dirty WHERE subject='declared'",[]).unwrap();
    drop(connection);
    (store,now)
}

#[test]
fn mission_metrics_wait_mapping_ages_labels_and_unknown_clock_visibility() {
    let (store,now) = waiting_store();
    let snapshot = store.flow_snapshot(now as u128).unwrap();
    assert_eq!(snapshot.mission_wait.counts,[2,3,2]);
    assert_eq!(snapshot.mission_wait.oldest_seconds,[1800.0;3]);
    assert_eq!(snapshot.mission_wait.unknown,0);
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder().with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
    let meter = provider.meter("st3");
    let sample = crate::otel::register_work_metrics_on(meter.clone(),Arc::new(store));
    crate::otel::register_mission_wait_metrics_on(meter,sample);
    provider.force_flush().unwrap();
    let finished = exporter.get_finished_metrics().unwrap();
    let mut names = BTreeSet::new();
    for metric in finished.iter().flat_map(|r|r.scope_metrics()).flat_map(|s|s.metrics())
        .filter(|metric|metric.name().starts_with("st.mission.steps."))
    {
        names.insert(metric.name().to_owned());
        match metric.data() {
            AggregatedMetrics::U64(MetricData::Gauge(gauge)) => {
                if metric.name()=="st.mission.steps.wait_unknown" {
                    for point in gauge.data_points() {
                        assert_eq!(point.value(),0);
                        assert_eq!(point.attributes().count(),0);
                    }
                } else {
                    assert_eq!(gauge.data_points().map(|point|point.value()).sum::<u64>(),7);
                    for point in gauge.data_points() { wait_labels(point.attributes()); }
                }
            }
            AggregatedMetrics::F64(MetricData::Gauge(gauge)) => {
                assert_eq!(metric.unit(),"s");
                assert_eq!(gauge.data_points().count(),3);
                for point in gauge.data_points() {
                    assert!(point.value()>=1800.0);
                    wait_labels(point.attributes());
                }
            }
            other => panic!("unexpected mission gauge: {other:?}"),
        }
    }
    assert_eq!(names.len(),3);
    provider.shutdown().unwrap();
    for unavailable in ["missing","dirty","pending","attempt"] {
        let (store,now) = waiting_store();
        {
            let connection = store.connection.write();
            match unavailable {
                "missing" => { connection.execute("DELETE FROM local_step_clocks WHERE subject='declared'",[]).unwrap(); }
                "dirty" => { connection.execute("INSERT OR IGNORE INTO local_step_clock_dirty VALUES('declared')",[]).unwrap(); }
                "pending" => { connection.execute("INSERT OR IGNORE INTO local_step_clock_pending VALUES('pending','declared')",[]).unwrap(); }
                _ => { connection.execute("UPDATE local_step_clocks SET attempt=2 WHERE subject='declared'",[]).unwrap(); }
            }
            let snapshot = store.flow_snapshot(now as u128).unwrap();
            assert_eq!(snapshot.mission_wait.unknown,1,"{unavailable}");
            let exporter = InMemoryMetricExporter::default();
            let provider = SdkMeterProvider::builder()
                .with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
            crate::otel::register_mission_wait_metrics_on(provider.meter("st3"),
                Arc::new(move || Some(snapshot.clone())));
            provider.force_flush().unwrap();
            let finished = exporter.get_finished_metrics().unwrap();
            let metrics = finished.iter().flat_map(|r|r.scope_metrics())
                .flat_map(|s|s.metrics()).collect::<Vec<_>>();
            let unknown = metrics.iter().find(|metric|metric.name()=="st.mission.steps.wait_unknown").unwrap();
            let AggregatedMetrics::U64(MetricData::Gauge(gauge)) = unknown.data()
                else { panic!("unknown clock gauge") };
            assert_eq!(gauge.data_points().next().unwrap().value(),1);
            for metric in metrics.iter().filter(|metric|metric.name()!="st.mission.steps.wait_unknown") {
                match metric.data() {
                    AggregatedMetrics::U64(MetricData::Gauge(gauge)) => assert_eq!(gauge.data_points().count(),0),
                    AggregatedMetrics::F64(MetricData::Gauge(gauge)) => assert_eq!(gauge.data_points().count(),0),
                    other => panic!("unexpected omitted gauge: {other:?}"),
                }
            }
            provider.shutdown().unwrap();
        }
    }
}

fn wait_labels<'a>(labels: impl Iterator<Item=&'a opentelemetry::KeyValue>) {
    let labels = labels.collect::<Vec<_>>();
    assert_eq!(labels.len(),1);
    assert_eq!(labels[0].key.as_str(),"st.mission.wait_reason");
    assert!(WAIT_REASONS.contains(&labels[0].value.as_str().as_ref()));
}
