use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde_json::json;

// Shorten only this publisher's lease. Deliberately hold API responses past it to model a busy
// daemon without depending on scheduler luck or changing a process-wide environment variable.
const LEASE_MS: u64 = 100;
const API_DELAY: Duration = Duration::from_millis(2 * LEASE_MS);

#[derive(Default)]
struct DelayedApi {
    last_response_at: AtomicU64,
    claims: Mutex<Vec<(ClaimInput, u64)>>,
}

async fn work(State(api): State<Arc<DelayedApi>>) -> Json<Value> {
    tokio::time::sleep(API_DELAY).await;
    api.last_response_at
        .store(ledger::now_ms(), Ordering::SeqCst);
    Json(json!({"api_version": "fractal.v1", "value": []}))
}

async fn claim(State(api): State<Arc<DelayedApi>>, Json(input): Json<ClaimInput>) -> Json<Value> {
    let boundary = api.last_response_at.load(Ordering::SeqCst);
    api.claims.lock().unwrap().push((input.clone(), boundary));
    if input.fields["subagent_id"] == "first" {
        // Later siblings must receive their own issuance time, including during renewal.
        tokio::time::sleep(API_DELAY).await;
    }
    api.last_response_at
        .store(ledger::now_ms(), Ordering::SeqCst);
    Json(json!({"api_version": "fractal.v1", "value": {
        "id": "claim/test", "store_index": 1, "batch_id": "batch/test",
        "subject": input.subject, "kind": input.kind, "origin": "lease-test",
        "actor": input.actor, "body": {"fields": input.fields}, "predecessors": [],
        "accepted_at_unix_ms": ledger::now_ms(),
    }}))
}

fn assert_fresh_leases(api: &DelayedApi, kind: &str) {
    let claims = api.claims.lock().unwrap();
    let matching = claims
        .iter()
        .filter(|(input, _)| input.kind == kind)
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 2, "one lease for each sibling: {claims:?}");
    for (input, boundary) in matching {
        let expires = input.fields["lease_expires_at_unix_ms"].as_u64().unwrap();
        assert!(
            expires >= boundary + LEASE_MS,
            "{} {} used time from before the preceding API response: expiry {expires}, response {boundary}",
            input.kind,
            input.fields["subagent_id"],
        );
    }
}

#[tokio::test]
async fn leases_start_after_attribution_and_each_preceding_claim_response() {
    let api = Arc::new(DelayedApi::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::new(crate::client::Endpoint::Http(format!(
        "http://{}",
        listener.local_addr().unwrap(),
    )));
    let app = Router::new()
        .route("/v1/work", get(work))
        .route("/v1/claims", post(claim))
        .with_state(api.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let mut publisher = Publisher::start(
        "agent/example/worker",
        "claude",
        "incarnation-1",
        root.path(),
        ledger::now_ms(),
    );
    publisher.lease_ms = LEASE_MS;
    for id in ["first", "second"] {
        ledger::observe_claude(
            root.path(),
            "SubagentStart",
            &json!({
                "session_id": "session-1", "agent_id": id, "agent_type": "Explore",
            }),
        )
        .unwrap();
    }
    publisher.tick(&client).await.unwrap();
    assert_fresh_leases(&api, "subagent.appeared");

    api.claims.lock().unwrap().clear();
    // Both leases are due; delay the first renewal response, just as the appearance above.
    publisher
        .published
        .open
        .values_mut()
        .for_each(|lease| *lease = 0);
    publisher.tick(&client).await.unwrap();
    assert_fresh_leases(&api, "subagent.renewed");
    server.abort();
}
