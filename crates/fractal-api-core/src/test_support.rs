//! Process-local fixture tools. The tools that need only the store live in fractal-store and are
//! re-exported here; the API fixtures live in this file.

pub use fractal_reconcile::test_support::*;

/// Runs this process's client-facing clock `skew_ms` ahead of the wall clock (0 restores it), so a
/// test can pass a real freshness horizon without sleeping. It is process-wide: use it only in a
/// test that nextest runs in its own process, and restore the clock before asserting on new reports.
#[cfg(feature = "test-support")]
pub fn skew_client_clock(skew_ms: u64) {
    crate::api::set_client_clock_skew_ms(skew_ms);
}

/// A message sent through `store`: a claim that node writes, and one no published list reads.
pub fn note_local_claim(store: &crate::store::Store, subject: &str) {
    store
        .append_claim(&crate::model::ClaimInput {
            subject: subject.into(),
            kind: "message.sent".into(),
            actor: Some("person/example".into()),
            fields: serde_json::from_value(serde_json::json!({"from": "person/example", "to": "agent/example",
                "status": "sent", "content": "a write someone waits to see"}))
            .unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

/// Progress on a step this node writes: a claim the work list's check reads (a `step-run/`
/// subject), and one the missions list's does not when no step of that name exists.
pub fn note_local_progress(store: &crate::store::Store, step: &str) {
    store
        .append_claim(&crate::model::ClaimInput {
            subject: step.into(),
            kind: "work.progress".into(),
            actor: Some("person/example".into()),
            fields: serde_json::from_value(serde_json::json!({"summary": "a step moved"})).unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

/// A resource observation this node writes: a claim the missions list's check reads, as it is
/// not one of the kinds that check counts as neutral.
pub fn note_local_resource(store: &crate::store::Store, subject: &str) {
    store
        .append_claim(&crate::model::ClaimInput {
            subject: subject.into(),
            kind: "resource.observed".into(),
            actor: Some("person/example".into()),
            fields: serde_json::from_value(serde_json::json!({"kind": "custom.example.notification", "facts": {}})).unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
}

/// The fleet [`fleet_member`] binds every member to.
pub const PAUSE_FLEET: &str = "6c5c2b9f-8d4e-4f3a-8b2c-1d7e6f5a4b3c";

/// One member of a fleet, at `dir`, as a peer that [`replicate`] reaches.
pub fn fleet_member(dir: &std::path::Path, name: &str) -> std::sync::Arc<crate::store::Store> {
    std::fs::create_dir_all(dir).unwrap();
    let store = crate::store::Store::open(&dir.join("graph.db"), name).unwrap();
    store.bind_fleet(PAUSE_FLEET).unwrap();
    std::sync::Arc::new(store)
}

/// Everything `from` holds, admitted by `to` the way a peer exchange does it.
pub fn replicate(from: &crate::store::Store, to: &crate::store::Store) {
    let exchange = from.export_replication_exchange(PAUSE_FLEET, &to.replication_inventory().unwrap()).unwrap();
    to.receive_replication_exchange(from.origin(), PAUSE_FLEET, &exchange).unwrap();
    to.validate_replication_backlog().unwrap();
    to.apply_replication_repairs().unwrap();
    assert!(to.project_replication_backlog().unwrap());
}

/// Only the fixture executable initializes the process-local login shell. Even a production
/// executable built with `test-support` cannot select this ingress behavior using a header,
/// environment variable, config setting, or executable name.
#[cfg(feature = "test-support")]
pub(crate) fn fixture_agent_router(
    app: axum::Router,
    store: std::sync::Arc<crate::store::Store>,
) -> axum::Router {
    if login_shell().is_none() {
        return app;
    }
    app.layer(axum::middleware::from_fn_with_state(store, bind_fixture_agent))
}

#[cfg(feature = "test-support")]
async fn bind_fixture_agent(
    axum::extract::State(store): axum::extract::State<std::sync::Arc<crate::store::Store>>,
    mut request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use crate::SeatsStoreCore as _;

    if let Some(agent) = request.headers().get(crate::api::client_v0::LOCAL_PERSON_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|actor| actor.starts_with("agent/"))
        .map(str::to_owned)
    {
        let subject = agent.clone();
        let declared = tokio::task::spawn_blocking(move || {
            store.desired_subjects_named(&[subject]).map(|subjects| {
                subjects.iter().any(|subject| subject.kind == "agent")
            })
        }).await;
        match declared {
            Ok(Ok(true)) => {
                request.extensions_mut().insert(crate::api::BoundAgent(agent.clone()));
                request.extensions_mut().insert(crate::api::HarnessAncestry::Bound(agent));
            }
            Ok(Ok(false)) => return (
                axum::http::StatusCode::FORBIDDEN,
                "fixture identity must name a declared agent",
            ).into_response(),
            _ => return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "cannot resolve fixture agent declaration",
            ).into_response(),
        }
    }
    next.run(request).await
}
