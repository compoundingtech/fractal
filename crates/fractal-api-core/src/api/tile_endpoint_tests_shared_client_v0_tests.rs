#![allow(unused_imports)]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use axum::extract::{Extension, Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::{Json, Router};
use serde_json::{Value, json};
use crate::api::{ApiError, AppState, Body, MessageRead, MessagesQuery, blocking_api, blocking_store, normalize_message_party};
use crate::model::{MessageSendReceipt, MessageSendRequest, MessageView};
use crate::tile_registry::{DEFAULT_REGISTRY, Registry, Tile};
use crate::api::tile_endpoint::*;
use std::io::{Read as _, Write as _};
use std::sync::Mutex;

pub fn caller() -> TileCaller {
        TileCaller {
            tile: "atlas".into(),
            agent: "agent/fleet/fixture-example/atlas".into(),
            person: "person/ada".into(),
            pid: 7,
        }
    }

pub async fn call(
        app: &Router,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
            .unwrap();
        request.extensions_mut().insert(caller());
        let response = tower::ServiceExt::oneshot(app.clone(), request)
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

pub fn message(from: &str, to: &str, key: &str) -> Value {
        json!({"idempotency_key": key, "from": from, "to": to, "content": "hello"})
    }

/// One raw HTTP request over the socket; `None` when the endpoint closed without a reply.
    pub fn get(socket: &Path, path: &str) -> Option<String> {
        let mut stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let _ = write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: tile\r\nConnection: close\r\n\r\n"
        );
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        (!response.is_empty()).then_some(response)
    }
