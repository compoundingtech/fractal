use super::*;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

#[test]
fn attention_metrics_reuse_canonical_open_rows_including_cancelled_run_updates() {
    let (store, origin, mut request) = crate::store::person_work::tests::fixture();
    assert!(store.attention_metrics("person/avery", now_ms()).is_none());
    store.start_attention_list_refresher().unwrap();
    store.ask_person(&request).unwrap();
    request.step = None;
    request.incarnation = None;
    request.idempotency_key = "update-metrics".into();
    request.request = Some(json!({"version":1,"type":"update","about":origin.run}));
    let update = store.ask_person(&request).unwrap();
    store.set_mission_run_state(&origin.run,"cancelled","terminal",Some("stopped")).unwrap();
    crate::api::refresh_attention_list(&store).unwrap();
    let publication = store.newest_attention_list().1.unwrap();
    let notice = publication.rows.iter().find(|row| row["source_id"]==update.subject).unwrap();
    assert_eq!(notice["state"],"open");
    assert!(notice.get("update").is_some());
    let now = now_ms()+5000;
    let scope = smallclaims::sqlite::work::SqliteWorkScope::start();
    let value = store.attention_metrics("person/avery", now).unwrap();
    let cost = scope.finish();
    assert_eq!(cost.vm_steps,0,"attention metrics must not execute SQL");
    assert_eq!(value.counts[1],1);
    assert!(value.oldest_seconds[1]>=5.0);
    assert_eq!(store.attention_metrics("person/not-configured",now).unwrap().counts,[0;2]);
    store.finish_person_step(&crate::model::PersonStepResponse {
        delegation:None, subject:update.subject, actor:"person/avery".into(), summary:"Read".into(),
        evidence:vec![], episode:None, idempotency_key:"read-update-metrics".into(),
        answer:Some(json!({"id":"read"})),
    },false).unwrap();
    crate::api::refresh_attention_list(&store).unwrap();
    assert_eq!(store.attention_metrics("person/avery",now).unwrap().counts[1],0);
}

#[test]
fn attention_metrics_export_only_configured_person_and_two_kinds() {
    let (store, _, request) = crate::store::person_work::tests::fixture();
    store.start_attention_list_refresher().unwrap();
    store.ask_person(&request).unwrap();
    crate::api::refresh_attention_list(&store).unwrap();
    let store = Arc::new(store);
    let expected = store.attention_metrics("person/avery",now_ms()).unwrap().counts;
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder().with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
    crate::otel::register_attention_metrics_on(provider.meter("st3"),store,"person/avery".into());
    provider.force_flush().unwrap();
    let finished = exporter.get_finished_metrics().unwrap();
    let mut instruments = 0;
    for metric in finished.iter().flat_map(|r|r.scope_metrics()).flat_map(|s|s.metrics()) {
        instruments += 1;
        match metric.data() {
            AggregatedMetrics::U64(MetricData::Gauge(gauge)) => {
                assert_eq!(metric.name(),"st.attention.open");
                assert_eq!(gauge.data_points().count(),2);
                assert_eq!(gauge.data_points().map(|point|point.value()).sum::<u64>(),expected.iter().sum());
                for point in gauge.data_points() { check_labels(point.attributes()); }
            }
            AggregatedMetrics::F64(MetricData::Gauge(gauge)) => {
                assert_eq!(metric.name(),"st.attention.oldest_age");
                assert_eq!(metric.unit(),"s");
                assert_eq!(gauge.data_points().count(),2);
                for point in gauge.data_points() { check_labels(point.attributes()); }
            }
            other => panic!("unexpected metric: {other:?}"),
        }
    }
    assert_eq!(instruments,2);
    provider.shutdown().unwrap();
}

fn check_labels<'a>(labels: impl Iterator<Item=&'a opentelemetry::KeyValue>) {
    let labels = labels.collect::<Vec<_>>();
    assert_eq!(labels.len(),2);
    for label in labels {
        match label.key.as_str() {
            "st.person" => assert_eq!(label.value.as_str(),"person/avery"),
            "st.attention.kind" => assert!(ATTENTION_KINDS.contains(&label.value.as_str().as_ref())),
            other => panic!("unbounded label key: {other}"),
        }
    }
}
