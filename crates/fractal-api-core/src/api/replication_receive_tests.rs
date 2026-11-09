use super::*;
use crate::api::client_push::tests::LogBuffer;
use tracing::instrument::WithSubscriber as _;

fn state(root: &std::path::Path) -> AppState {
    AppState {
        seat_admission: Default::default(),
        store: Arc::new(Store::open(&root.join("graph.db"), "summary-target").unwrap()),
        notify: Arc::new(Notify::new()),
        event_notify: watch::channel(0u64).0,
        node: "summary-target".into(),
        state_dir: root.into(),
        pty_root: root.join("pty"),
        pty_binary: root.join("unused"),
        fleet_id: None,
        configured_peers: vec![],
        client_relay: None,
        native_session_home: None,
        planner_default: Default::default(),
        harness_question_sink: None,
        owning_person: None,
        assistant_receipts: Default::default(),
        private_notes: Default::default(),
    }
}

#[tokio::test]
async fn replication_receive_logs_one_summary_with_expected_fields_on_success_and_error() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path());
    let source = Store::open_memory("summary-source").unwrap();
    let exchange = source.export_replication_summary("summary-fleet").unwrap();
    let request = ReplicationReceiveRequest {
        peer: exchange.peer.clone(),
        fleet_id: exchange.fleet_id.clone(),
        exchange,
        round_trip_ms: None,
        carrier: Default::default(),
    };
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .without_time()
        .with_writer(logs.clone())
        .finish();
    async {
        for (index, correlation) in [None, Some("exchange-test"), Some("exchange-error")].into_iter().enumerate() {
            let mut headers = axum::http::HeaderMap::new();
            if let Some(correlation) = correlation {
                headers.insert("x-fractal-exchange-id", correlation.parse().unwrap());
            }
            let mut request = request.clone();
            if index == 2 {
                request.exchange.peer = "mismatched-label".into();
            }
            let result = replication_receive(State(state.clone()), headers, Json(request)).await;
            if index == 2 {
                assert_eq!(result.unwrap_err().code, "peer-label-mismatch");
            } else {
                assert!(result.is_ok(), "{result:?}");
            }
            let bytes = logs.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let captured = String::from_utf8_lossy(&bytes);
            let summaries: Vec<_> = captured.lines()
                .filter(|line| line.contains("replication receive summary"))
                .collect();
            assert_eq!(summaries.len(), index + 1, "{captured}");
            let summary = summaries[index];
            assert!(summary.contains(" INFO "), "{summary}");
            for field in [
                "correlation_id", "peer", "envelopes_offered", "envelopes_admitted",
                "admit_ms", "project_ms", "projection_chunks", "rebuild_part_count",
                "rebuild_part_ms", "commits", "total_ms", "outcome",
            ] {
                assert!(summary.contains(&format!("{field}=")), "missing {field}: {summary}");
            }
            assert!(summary.contains(&format!("correlation_id={}", correlation.unwrap_or("-"))), "{summary}");
            assert!(summary.contains("peer=summary-source"), "{summary}");
            assert!(summary.contains("envelopes_offered=0"), "{summary}");
            assert!(summary.contains("envelopes_admitted=0"), "{summary}");
            assert!(summary.contains(if index == 2 { "outcome=peer-label-mismatch" } else { "outcome=ok" }), "{summary}");
        }
    }.with_subscriber(subscriber).await;
}
