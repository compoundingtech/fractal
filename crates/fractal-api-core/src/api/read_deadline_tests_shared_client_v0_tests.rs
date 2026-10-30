#![allow(unused_imports)]
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::sync::Arc;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use axum::Json;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use smallclaims::read_budget::{self, ReadBudget};
use crate::api::{ApiError, AppState, ClientTransportBoundary};
use crate::api::read_deadline::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn request(method: Method, path: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap()
    }

/// The handler idles for the longest wait on the paused clock (callers run `start_paused`), so
    /// the ordinary 15 s deadline still has to give way to the 30 s wait, without 30 real seconds.
    /// The handler runs on a blocking poll, which keeps the paused clock from advancing by itself,
    /// so the test advances it once the handler has started.
    pub async fn idle_long_poll_keeps_its_normal_empty_answer(
        path: &'static str,
        route: &'static str,
        paused: bool,
    ) {
        use axum::{Router, middleware::from_fn_with_state, routing::get};
        use tower::ServiceExt;
        let root = tempfile::tempdir().unwrap();
        let state = crate::api::tests_shared::state(root.path());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started = Arc::new(std::sync::Mutex::new(Some(started_tx)));
        let app = Router::new()
            .route(
                route,
                get(move || {
                    let started = started.clone();
                    async move {
                        // The wait starts counting when it is created, so create it before
                        // signalling: the test advances the clock as soon as it hears.
                        // On the real clock, just past the ordinary deadline proves it gave way;
                        // the paused tests idle for the whole wait at no cost.
                        let idle = if paused {
                            Duration::from_millis(MAX_WAIT_MS)
                        } else {
                            ORDINARY + Duration::from_secs(1)
                        };
                        let wait = tokio::time::sleep(idle);
                        let _ = started.lock().unwrap().take().unwrap().send(());
                        wait.await;
                        Json(json!({ "items": [] }))
                    }
                }),
            )
            .layer(from_fn_with_state(
                (state, ClientTransportBoundary::Unix),
                crate::api::response_envelope,
            ));
        let response = tokio::spawn(app.oneshot(request(Method::GET, path)));
        started_rx.await.unwrap();
        if paused {
            let past_ordinary = ORDINARY + Duration::from_secs(1);
            tokio::time::advance(past_ordinary).await;
            assert!(!response.is_finished(), "{path} was cut at the ordinary deadline");
            tokio::time::advance(Duration::from_millis(MAX_WAIT_MS) - past_ordinary).await;
        }
        let response = response.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let value = body.get("value").unwrap_or(&body);
        assert_eq!(value["items"], json!([]), "{body}");
    }
