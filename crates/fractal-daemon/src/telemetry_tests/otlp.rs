//! The OTLP exporter's tests that read the local observation log from a store.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::otlp::*;
use crate::store::Store;
use crate::model::ClaimInput;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU16, Ordering};

#[derive(Clone, Default)]
struct Collector {
    requests: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    status: Arc<AtomicU16>,
    metrics_status: Arc<AtomicU16>,
    traces_status: Arc<AtomicU16>,
}

async fn collect(
    State(collector): State<Collector>,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> StatusCode {
    let signal_status = match uri.path() {
        "/v1/metrics" => collector.metrics_status.load(Ordering::SeqCst),
        "/v1/traces" => collector.traces_status.load(Ordering::SeqCst),
        _ => 0,
    };
    let status = StatusCode::from_u16(if signal_status == 0 {
        collector.status.load(Ordering::SeqCst)
    } else {
        signal_status
    })
    .unwrap_or(StatusCode::OK);
    if status.is_success() {
        collector.requests.lock().unwrap().push((headers, body));
    }
    status
}

async fn start_collector() -> (Collector, String) {
    let collector = Collector::default();
    collector.status.store(200, Ordering::SeqCst);
    let app = Router::new()
        .route("/v1/logs", post(collect))
        .route("/v1/metrics", post(collect))
        .route("/v1/traces", post(collect))
        .with_state(collector.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (collector, format!("http://{address}/"))
}

fn observe(store: &Store, entry: u64) {
    store
        .append_claim(&ClaimInput {
            subject: "agent/node.worker".into(),
            kind: "harness.timeline".into(),
            actor: Some("agent/node.worker".into()),
            fields: BTreeMap::from([
                ("operation".into(), Value::String("append".into())),
                ("entry_id".into(), Value::String(format!("entry-{entry}"))),
                ("sequence".into(), Value::from(entry)),
                ("revision".into(), Value::from(1)),
                ("role".into(), Value::String("assistant".into())),
                ("entry_type".into(), Value::String("content".into())),
                ("final".into(), Value::Bool(true)),
                (
                    "body".into(),
                    json!({"media_type": "text/plain", "text": format!("entry {entry}")}),
                ),
                ("driver".into(), Value::String("codex".into())),
                ("incarnation_id".into(), Value::String("inc-1".into())),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("otlp-entry-{entry}")),
        })
        .unwrap();
}

fn records(request: &Value) -> Vec<Value> {
    request["resourceLogs"][0]["scopeLogs"][0]["logRecords"]
        .as_array()
        .unwrap()
        .clone()
}

fn resource_service_name(request: &Value) -> &str {
    request["resourceLogs"]
        .get(0)
        .or_else(|| request["resourceMetrics"].get(0))
        .or_else(|| request["resourceSpans"].get(0))
        .and_then(|resource| {
            resource["resource"]["attributes"]
                .as_array()
                .and_then(|attributes| {
                    attributes
                        .iter()
                        .find(|attribute| attribute["key"] == "service.name")
                        .and_then(|attribute| attribute["value"]["stringValue"].as_str())
                })
        })
        .unwrap()
}

#[test]
fn a_local_observation_becomes_an_otlp_log_record() {
    let store = Store::open_memory("node-a").unwrap();
    observe(&store, 7);
    let batch = store.local_observations_after(0, 10).unwrap();
    let request = otlp_logs("node-a", &batch);
    let resource = &request["resourceLogs"][0]["resource"]["attributes"];
    assert!(resource.as_array().unwrap().contains(&json!({
        "key": "service.name", "value": {"stringValue": "st-daemon"}
    })));
    assert!(resource.as_array().unwrap().contains(&json!({
        "key": "st3.node", "value": {"stringValue": "node-a"}
    })));
    let record = &records(&request)[0];
    assert_eq!(record["eventName"], "harness.timeline");
    assert_eq!(
        record["timeUnixNano"],
        (batch[0].accepted_at_unix_ms * 1_000_000).to_string()
    );
    let attributes = record["attributes"].as_array().unwrap();
    for (key, value) in [
        ("st3.subject", json!({"stringValue": "agent/node.worker"})),
        ("st3.kind", json!({"stringValue": "harness.timeline"})),
        ("st3.actor", json!({"stringValue": "agent/node.worker"})),
        ("st3.incarnation_id", json!({"stringValue": "inc-1"})),
        ("st3.local_id", json!({"intValue": "1"})),
    ] {
        assert!(
            attributes.contains(&json!({"key": key, "value": value})),
            "{key}"
        );
    }
    let body = record["body"]["kvlistValue"]["values"].as_array().unwrap();
    assert!(body.contains(&json!({"key": "sequence", "value": {"intValue": "7"}})));
    assert!(body.contains(&json!({"key": "final", "value": {"boolValue": true}})));
    assert!(body.contains(&json!({
        "key": "body",
        "value": {"kvlistValue": {"values": [
            {"key": "media_type", "value": {"stringValue": "text/plain"}},
            {"key": "text", "value": {"stringValue": "entry 7"}}
        ]}}
    })));
}

#[tokio::test]
async fn each_model_response_exports_its_spend_as_a_log_and_bounded_counters() {
    let (collector, endpoint) = start_collector().await;
    let store = Arc::new(Store::open_memory("node-a").unwrap());
    let respond = |entry: u64, account: Option<&str>| {
        let mut body = json!({"semantics": "response", "model": "claude-opus-5-5",
            "input_tokens": 1000, "output_tokens": 100, "cached_tokens": 10000,
            "cache_write_tokens": 0, "total_tokens": 11100});
        if let Some(account) = account {
            body["account"] = json!(account);
        }
        store
            .append_claim(&ClaimInput {
                subject: "agent/node.worker".into(),
                kind: "harness.timeline".into(),
                actor: Some("agent/node.worker".into()),
                fields: BTreeMap::from([
                    ("operation".into(), json!("append")),
                    ("entry_id".into(), json!(format!("usage-{entry}"))),
                    ("source_id".into(), json!(format!("source-{entry}"))),
                    ("sequence".into(), json!(entry)),
                    ("revision".into(), json!(1)),
                    ("role".into(), json!("system")),
                    ("entry_type".into(), json!("usage")),
                    ("final".into(), json!(true)),
                    ("body".into(), body),
                    ("driver".into(), json!("claude")),
                    ("incarnation_id".into(), json!("inc-1")),
                    (
                        "observed_at_unix_ms".into(),
                        json!(1_700_000_000_000_u64 + entry),
                    ),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("otlp-usage-{entry}")),
            })
            .unwrap();
    };
    respond(1, Some("claude/aaaaaaaaaaaaaaaa"));
    respond(2, Some("claude/aaaaaaaaaaaaaaaa"));
    respond(3, None);
    let exporter = OtlpExporter::new(
        &OtlpConfig {
            endpoint,
            headers_file: None,
        },
        "node-a",
    )
    .unwrap();
    assert_eq!(exporter.export_once(&store).await.unwrap().exported, 3);
    let requests = collector.requests.lock().unwrap();
    let usage = records(&requests[0].1)
        .into_iter()
        .filter(|record| record["eventName"] == "st.usage.response")
        .collect::<Vec<_>>();
    assert_eq!(usage.len(), 3);
    assert_eq!(usage[0]["timeUnixNano"], "1700000000001000000");
    let attributes = usage[0]["attributes"].as_array().unwrap();
    // Opus 5.5: $4 input, $20 output and $0.20 cache reads per million tokens.
    for (key, value) in [
        ("st.agent", json!({"stringValue": "agent/node.worker"})),
        ("st.model", json!({"stringValue": "claude-opus-5-5"})),
        (
            "st.account",
            json!({"stringValue": "claude/aaaaaaaaaaaaaaaa"}),
        ),
        ("st.basis", json!({"stringValue": "estimated"})),
        ("st.step", json!({"stringValue": "unknown"})),
        ("st.tokens.cache_read", json!({"intValue": "10000"})),
        ("st.cost.microusd", json!({"intValue": "8000"})),
    ] {
        assert!(
            attributes.contains(&json!({"key": key, "value": value})),
            "{key}: {attributes:#?}"
        );
    }
    let usage_body = requests
        .iter()
        .find(|(_, body)| body.get("resourceMetrics").is_some())
        .map(|(_, body)| body)
        .unwrap();
    assert_eq!(
        resource_service_name(usage_body),
        "st-daemon",
        "usage counters are daemon bookkeeping, not per-hook signals"
    );
    let metrics = &usage_body["resourceMetrics"][0]["scopeMetrics"][0]["metrics"];
    let cost = metrics
        .as_array()
        .unwrap()
        .iter()
        .find(|metric| metric["name"] == "st_usage_cost_microusd_total")
        .unwrap();
    let points = cost["sum"]["dataPoints"].as_array().unwrap();
    assert_eq!(points.len(), 2, "one point per account: {points:#?}");
    assert!(points.iter().any(|point| point["asInt"] == "16000"));
    let labels = points[0]["attributes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|attribute| attribute["key"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(labels, ["driver", "model", "account", "basis"]);
}

#[tokio::test]
async fn the_exporter_delivers_each_observation_once_the_collector_accepts_it() {
    let (collector, endpoint) = start_collector().await;
    let directory = tempfile::tempdir().unwrap();
    let headers_file = directory.path().join("headers.toml");
    std::fs::write(&headers_file, "x-api-key = \"example-key\"\n").unwrap();
    let store = Arc::new(Store::open_memory("node-a").unwrap());
    let exporter = OtlpExporter::new(
        &OtlpConfig {
            endpoint,
            headers_file: Some(headers_file),
        },
        "node-a",
    )
    .unwrap();
    assert_eq!(
        exporter.export_once(&store).await.unwrap(),
        ExportOutcome::default()
    );
    for entry in 1..=3 {
        observe(&store, entry);
    }
    assert_eq!(
        exporter.export_once(&store).await.unwrap(),
        ExportOutcome {
            exported: 3,
            skipped: 0
        }
    );
    assert_eq!(store.otlp_export_cursor().unwrap(), 3);
    {
        let requests = collector.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0["x-api-key"], "example-key");
        assert_eq!(records(&requests[0].1).len(), 3);
    }
    assert_eq!(
        exporter.export_once(&store).await.unwrap().exported,
        0,
        "nothing is sent twice"
    );

    collector.status.store(503, Ordering::SeqCst);
    observe(&store, 4);
    assert!(exporter.export_once(&store).await.is_err());
    assert_eq!(
        store.otlp_export_cursor().unwrap(),
        3,
        "a refused batch is sent again"
    );
    collector.status.store(200, Ordering::SeqCst);
    assert_eq!(exporter.export_once(&store).await.unwrap().exported, 1);
    assert_eq!(store.otlp_export_cursor().unwrap(), 4);

    observe(&store, 5);
    observe(&store, 6);
    // Keep only the newest observation, as a trim past the export cursor would.
    store.trim_local_observations(0, 1, 100).unwrap();
    assert_eq!(
        exporter.export_once(&store).await.unwrap(),
        ExportOutcome {
            exported: 1,
            skipped: 1
        }
    );
    assert_eq!(store.otlp_export_cursor().unwrap(), 6);
    assert_eq!(collector.requests.lock().unwrap().len(), 3);
}

fn observe_hook(store: &Store, event: &str) {
    store.append_claim(&ClaimInput {
        subject: "agent/example/seat".into(), kind: crate::telemetry::KIND.into(),
        actor: Some("agent/example/seat".into()),
        fields: BTreeMap::from([
            ("driver".into(), json!("claude")), ("unit".into(), json!("hook")),
            ("incarnation_id".into(), json!("incarnation-1")),
            ("signals".into(), json!({
                "started_at_unix_nano": "1000000000", "ended_at_unix_nano": "1025000000",
                "exit_code": 0,
                "hook_invocations": [{"hook": "claude-observe", "event": event}],
                "logs": [{"severity": "WARN", "fields": {"message": "example warning"}}],
            })),
        ]),
        evidence: Vec::new(), expected_subject: None, idempotency_key: None,
    }).unwrap();
}

#[tokio::test]
async fn hook_signals_carry_the_hook_service_and_do_not_advance_until_every_signal_is_accepted()
{
    let (collector, endpoint) = start_collector().await;
    let store = Arc::new(Store::open_memory("node-a").unwrap());
    observe_hook(&store, "PreToolUse");
    let batch = store.local_observations_after(0, 10).unwrap();
    assert_eq!(
        store.index().unwrap(),
        0,
        "telemetry never enters replicated history"
    );
    let exporter = OtlpExporter::new(
        &OtlpConfig {
            endpoint,
            headers_file: None,
        },
        "node-a",
    )
    .unwrap();
    collector.metrics_status.store(503, Ordering::SeqCst);
    assert!(exporter.export_once(&store).await.is_err());
    assert_eq!(store.otlp_export_cursor().unwrap(), 0);
    collector.metrics_status.store(200, Ordering::SeqCst);
    collector.traces_status.store(503, Ordering::SeqCst);
    assert!(exporter.export_once(&store).await.is_err());
    assert_eq!(store.otlp_export_cursor().unwrap(), 0);
    collector.traces_status.store(200, Ordering::SeqCst);
    assert_eq!(exporter.export_once(&store).await.unwrap().exported, 1);
    assert_eq!(store.otlp_export_cursor().unwrap(), 1);
    assert_eq!(exporter.export_once(&store).await.unwrap().exported, 0);
    let requests = collector.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        6,
        "a partial acceptance can repeat, without dropping a signal"
    );
    for (_, request) in requests.iter() {
        let expected = if request["resourceLogs"].get(0).is_some() {
            "st-daemon"
        } else {
            "st-hook"
        };
        assert_eq!(
            resource_service_name(request),
            expected,
            "hook spans and invocation counters carry st-hook; logs stay st-daemon"
        );
    }
    let logs = records(&requests[0].1);
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[1]["severityText"], "WARN");
    let expected_span = hook_span(&batch[0]).unwrap();
    assert_eq!(logs[1]["traceId"], expected_span["traceId"]);
    assert_eq!(logs[1]["spanId"], expected_span["spanId"]);
    let metrics = &requests[2].1["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0];
    assert_eq!(metrics["name"], "hook_invocations_total");
    assert_eq!(metrics["sum"]["aggregationTemporality"], 1);
    assert_eq!(metrics["sum"]["dataPoints"][0]["asInt"], "1");
    assert_eq!(
        metrics["sum"]["dataPoints"][0]["attributes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let spans = &requests[5].1["resourceSpans"][0]["scopeSpans"][0]["spans"];
    assert_eq!(spans, &json!([expected_span]));
}

#[test]
fn hook_metric_labels_are_bounded_and_a_non_application_has_no_counter_or_span() {
    let store = Store::open_memory("node-a").unwrap();
    observe_hook(&store, "arbitrary-new-event-with-an-identity");
    let mut batch = store.local_observations_after(0, 10).unwrap();
    let metric = otlp_hook_metrics("node-a", &batch).unwrap();
    assert_eq!(
        metric["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]["dataPoints"][0]["attributes"]
            [1]["value"]["stringValue"],
        "other"
    );
    batch[0].body["fields"]["signals"]["hook_invocations"] = json!([]);
    assert!(otlp_hook_metrics("node-a", &batch).is_none());
    assert!(otlp_hook_traces("node-a", &batch).is_none());
    assert_eq!(
        records(&otlp_logs("node-a", &batch))[1]["severityText"],
        "WARN"
    );
}

/// Every native harness reading reaches the collector as a log record, while the graph keeps
/// only the readings that change a current value; account limits also go out as a bounded
/// gauge. Exporting writes nothing to the graph.
#[tokio::test]
async fn every_harness_reading_exports_while_the_graph_keeps_current_values() {
    use crate::store::{append_latest_observation_publication, append_limits_publication};
    let (collector, endpoint) = start_collector().await;
    let store = Arc::new(Store::open_memory("node-a").unwrap());
    let (ada, bea) = ("agent/node.ada", "agent/node.bea");
    let reading = |subject: &str, kind: &str, fields: Value| ClaimInput {
        subject: subject.into(),
        kind: kind.into(),
        actor: Some(subject.into()),
        fields: serde_json::from_value(fields).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    };
    for subject in [ada, bea] {
        store
            .append_claim(&reading(subject, "runtime.observed", json!({"status": "running", "incarnation_id": "inc-1"})))
            .unwrap();
    }
    let start = smallclaims::store::now_ms();
    let state = |state: &str, at: u128| {
        append_latest_observation_publication(
            &store.graph,
            &reading(ada, "harness.observed", json!({"state": state, "driver": "claude",
                "incarnation_id": "inc-1", "observed_at_ms": at as u64})),
            at,
            Some("inc-1"),
        )
        .unwrap();
    };
    let limits = |subject: &str, five_hour: f64, at: u128| {
        append_limits_publication(
            &store.graph,
            &reading(subject, "harness.limits", json!({"driver": "claude", "account": "claude/example",
                "incarnation_id": "inc-1", "five_hour_percent": five_hour, "weekly_percent": 40.0,
                "measured_at_unix_ms": at as u64})),
            at,
            Some("inc-1"),
        )
        .unwrap();
    };
    state("idle", start);
    state("idle", start + 20_000);
    state("working", start + 40_000);
    state("idle", start + 50_000);
    limits(ada, 10.0, start);
    limits(bea, 10.0, start + 1_000);
    limits(ada, 12.0, start + 2_000);
    limits(bea, 12.0, start + 3_000);
    let graph = || {
        [ada, bea]
            .iter()
            .flat_map(|subject| ["harness.observed", "harness.limits"].map(|kind| (subject, kind)))
            .map(|(subject, kind)| store.claims_for(subject, Some(kind)).unwrap().len())
            .sum::<usize>()
    };
    assert_eq!(graph(), 6, "three state changes, each seat's first limits reading and the account's one change");
    // Once every member hears this node's seats, a working re-stamp past the five-minute
    // fallback stays local too, and still exports.
    store.set_fleet_hears_seats(true).unwrap();
    state("working", start + 60_000);
    state("working", start + 420_000);
    assert_eq!(graph(), 7, "the transition replicates; the re-stamp the fleet hears through its host stays local");

    let exporter = OtlpExporter::new(&OtlpConfig { endpoint, headers_file: None }, "node-a").unwrap();
    exporter.export_once(&store).await.unwrap();
    assert_eq!(graph(), 7, "exporting writes nothing to the graph");
    let requests = collector.requests.lock().unwrap();
    let logs = requests
        .iter()
        .filter(|(_, body)| body.get("resourceLogs").is_some())
        .flat_map(|(_, body)| records(body))
        .filter(|record| record["eventName"].as_str().is_some_and(|kind| kind.starts_with("harness.")))
        .count();
    assert_eq!(logs, 10, "the full series: every reading, including those the graph kept local");
    let metrics = requests
        .iter()
        .filter_map(|(_, body)| body["resourceMetrics"][0]["scopeMetrics"][0]["metrics"].as_array())
        .flatten()
        .collect::<Vec<_>>();
    let points = |name: &str| {
        metrics
            .iter()
            .find(|metric| metric["name"] == name)
            .map(|metric| metric["gauge"]["dataPoints"].as_array().unwrap().clone())
            .unwrap_or_else(|| panic!("{name} in {metrics:#?}"))
    };
    let labels = |point: &Value| {
        point["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|attribute| (attribute["key"].as_str().unwrap().to_owned(), attribute["value"]["stringValue"].as_str().unwrap().to_owned()))
            .collect::<Vec<_>>()
    };
    let limit = points("st_account_limit_used_percent")
        .iter()
        .map(|point| (labels(point)[2].1.clone(), point["asDouble"].as_f64().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(limit, [("five_hour".to_owned(), 12.0), ("weekly".to_owned(), 40.0)], "the newest reading of each window");
    let limit_point = &points("st_account_limit_used_percent")[0];
    assert_eq!(
        labels(limit_point).iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
        ["driver", "account", "window"],
        "no seat identity in metric labels"
    );
}

/// Claim writes are counted as they are stored, per kind and writing node, so the write rate
/// stays true when claims are deleted before an export (as local trim does). The totals are
/// cumulative from when counting started, and an export with nothing new stored sends none.
#[tokio::test]
async fn stored_claims_are_counted_when_stored_even_if_deleted_before_export() {
    let (collector, endpoint) = start_collector().await;
    let store = Arc::new(Store::open_memory("node-a").unwrap());
    let claim = |incarnation: &str| ClaimInput {
        subject: "agent/node.ada".into(),
        kind: "runtime.observed".into(),
        actor: Some("agent/node.ada".into()),
        fields: serde_json::from_value(json!({"status": "running", "incarnation_id": incarnation})).unwrap(),
        evidence: Vec::new(),
        expected_subject: None,
        idempotency_key: None,
    };
    let exporter = OtlpExporter::new(&OtlpConfig { endpoint, headers_file: None }, "node-a").unwrap();
    let stored = ["inc-1", "inc-2", "inc-3"].map(|incarnation| store.append_claim(&claim(incarnation)).unwrap());
    let superseded = stored[0].id.clone();
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h69066a2ded6a owner=fractal_daemon::telemetry_tests::otlp TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&superseded]).unwrap();
    assert!(store.claim_by_id(&superseded).unwrap().is_none(), "the claim is gone before the export");
    exporter.export_once(&store).await.unwrap();
    exporter.export_once(&store).await.unwrap();
    store.append_claim(&claim("inc-4")).unwrap();
    exporter.export_once(&store).await.unwrap();
    let requests = collector.requests.lock().unwrap();
    let series = |name: &str| {
        requests
            .iter()
            .filter_map(|(_, body)| body["resourceMetrics"][0]["scopeMetrics"][0]["metrics"].as_array())
            .flatten()
            .filter(|metric| metric["name"] == name)
            .map(|metric| {
                assert_eq!(metric["sum"]["aggregationTemporality"], 2, "cumulative");
                metric["sum"]["dataPoints"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|point| point["attributes"][0]["value"]["stringValue"] == "runtime.observed")
                    .map(|point| {
                        assert_eq!(point["attributes"][1]["value"]["stringValue"], "node-a", "the writing node");
                        (point["startTimeUnixNano"].as_str().unwrap().to_owned(), point["asInt"].as_str().unwrap().parse::<u64>().unwrap())
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let claims = series("st_claims_stored_total");
    assert_eq!(
        claims.iter().map(|points| points.iter().map(|(_, value)| *value).collect::<Vec<_>>()).collect::<Vec<_>>(),
        [vec![3], vec![4]],
        "the deleted claim still counts, the idle export sends nothing, and the total grows"
    );
    assert_eq!(claims[0][0].0, claims[1][0].0, "one start time for the cumulative series");
    let bytes = series("st_claim_body_bytes_stored_total");
    assert_eq!(bytes.len(), 2);
    let body = stored[1].body.to_string().len() as u64;
    assert!(bytes[0][0].1 >= 2 * body && bytes[1][0].1 > bytes[0][0].1, "{bytes:?} counts the three bodies of about {body} bytes, then a fourth");
}

/// The limits gauge carries each window's newest measured reading, even when an older reading
/// is appended after a newer one, in the same export or a later one.
#[tokio::test]
async fn the_limits_gauge_keeps_the_newest_measured_reading() {
    use crate::store::append_limits_publication;
    let (collector, endpoint) = start_collector().await;
    let store = Arc::new(Store::open_memory("node-a").unwrap());
    let start = smallclaims::store::now_ms();
    let reading = |subject: &str, percent: f64, measured: u128| {
        store
            .append_claim(&ClaimInput {
                subject: subject.into(),
                kind: "runtime.observed".into(),
                actor: Some(subject.into()),
                fields: serde_json::from_value(json!({"status": "running", "incarnation_id": "inc-1"})).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        append_limits_publication(
            &store.graph,
            &ClaimInput {
                subject: subject.into(),
                kind: "harness.limits".into(),
                actor: Some(subject.into()),
                fields: serde_json::from_value(json!({"driver": "claude", "account": "claude/example",
                    "incarnation_id": "inc-1", "five_hour_percent": percent, "measured_at_unix_ms": measured as u64}))
                .unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            },
            start + 120_000,
            Some("inc-1"),
        )
        .unwrap();
    };
    let exporter = OtlpExporter::new(&OtlpConfig { endpoint, headers_file: None }, "node-a").unwrap();
    let gauges = || {
        collector
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, body)| body["resourceMetrics"][0]["scopeMetrics"][0]["metrics"].as_array())
            .flatten()
            .filter(|metric| metric["name"] == "st_account_limit_used_percent")
            .map(|metric| {
                let point = &metric["gauge"]["dataPoints"][0];
                (point["asDouble"].as_f64().unwrap(), point["timeUnixNano"].as_str().unwrap().to_owned())
            })
            .collect::<Vec<_>>()
    };
    let at = |measured: u128| (measured * 1_000_000).to_string();
    reading("agent/node.ada", 30.0, start + 60_000);
    reading("agent/node.bea", 20.0, start);
    exporter.export_once(&store).await.unwrap();
    assert_eq!(gauges(), [(30.0, at(start + 60_000))], "the newer measurement, appended first");
    reading("agent/node.cara", 10.0, start + 30_000);
    exporter.export_once(&store).await.unwrap();
    assert_eq!(gauges().len(), 1, "an older reading in a later export sends no gauge");
    reading("agent/node.dan", 40.0, start + 90_000);
    exporter.export_once(&store).await.unwrap();
    assert_eq!(gauges().last(), Some(&(40.0, at(start + 90_000))), "a newer reading still updates it");
}

/// Diagnostics, which stay in the local log (C4 for daemon.diagnostic, and 2c for observability
/// codes of harness.diagnostic), export at their own severity with their code as an attribute, so
/// a collector can filter on both; a diagnostic without a severity stays INFO.
#[test]
fn diagnostics_export_at_their_severity_with_their_code() {
    let store = Store::open_memory("node-a").unwrap();
    let diagnostic = |subject: &str, kind: &str, actor: Option<&str>, fields: Value| {
        store.append_local_observations_for_test(&[ClaimInput {
            subject: subject.into(),
            kind: kind.into(),
            actor: actor.map(str::to_owned),
            fields: serde_json::from_value(fields).unwrap(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }]);
    };
    diagnostic("daemon/node-a", "daemon.diagnostic", None, json!({"severity":"error", "code":"reconcile-failed", "reason":"invented"}));
    diagnostic("daemon/node-a", "daemon.diagnostic", None, json!({"severity":"warning", "code":"kill-survivor", "reason":"invented"}));
    let batch = store.local_observations_after(0, 10).unwrap();
    let logs = otlp_logs("node-a", &batch);
    let records = records(&logs);
    let code = |record: &Value| {
        record["attributes"].as_array().unwrap().iter()
            .find(|attribute| attribute["key"] == "st3.code")
            .map(|attribute| attribute["value"]["stringValue"].clone())
    };
    assert_eq!((records[0]["severityText"].clone(), records[0]["severityNumber"].clone()), (json!("ERROR"), json!(17)));
    assert_eq!(code(&records[0]), Some(json!("reconcile-failed")));
    assert_eq!((records[1]["severityText"].clone(), records[1]["severityNumber"].clone()), (json!("WARN"), json!(13)));
    assert_eq!(code(&records[1]), Some(json!("kill-survivor")));
}
