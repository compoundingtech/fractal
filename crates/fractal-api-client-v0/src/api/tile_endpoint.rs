//! The part of `api::tile_endpoint` that moved up from fractal-api-core.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use axum::extract::{Extension, Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use super::{
    ApiError, AppState, Body, MessageRead, MessagesQuery, blocking_api, blocking_store,
    normalize_message_party,
};
use crate::model::{MessageSendReceipt, MessageSendRequest, MessageView};
use crate::tile_registry::{DEFAULT_REGISTRY, Registry, Tile};
pub use fractal_api_core::api::tile_endpoint::*;
pub use fractal_api_core::api::tile_endpoint::{Kernel, SOCKET, TileCaller, router, serve_listener, spawn, spawn_from};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::sync::Mutex;
    use fractal_api_core::api::tile_endpoint_tests_shared_client_v0_tests::{call, caller, get, message};

    #[tokio::test]
    async fn a_tile_acts_only_as_its_own_agent() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::api::tests_shared::state(root.path());
        let app = router(state.clone());
        let me = "agent/fleet/fixture-example/atlas";
        let other = "agent/fleet/fixture-example/birch";

        let (status, seen) = call(&app, "GET", "/v1/tile/whoami", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(seen["agent"], me);
        assert_eq!(seen["tile"], "atlas");

        // Sending as itself works, with or without naming itself.
        let (status, sent) =
            call(&app, "POST", "/v1/messages", Some(message(me, other, "a"))).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        assert_eq!(sent["from"], me);
        let (status, sent) =
            call(&app, "POST", "/v1/messages", Some(message("", other, "b"))).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        assert_eq!(sent["from"], me);
        // Naming anyone else is refused, not rewritten.
        for from in [other, "person/ada", "fleet/fixture-example/birch"] {
            let (status, refused) =
                call(&app, "POST", "/v1/messages", Some(message(from, me, "c"))).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{from}: {refused}");
            assert_eq!(refused["code"], "tile-forbidden");
        }
        let mut attached = message(me, other, "d");
        attached["attachments"] = json!([{"blob": "blob/x", "media_type": "image/png"}]);
        let (status, _) = call(&app, "POST", "/v1/messages", Some(attached)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Its own mailbox, never another's.
        let (status, mail) = call(&app, "GET", "/v1/messages", None).await;
        assert_eq!(status, StatusCode::OK, "{mail}");
        let (status, _) = call(&app, "GET", &format!("/v1/messages?to={other}"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // A message between others is not found; its own are.
        let between_others = {
            let state = state.clone();
            let request: MessageSendRequest =
                serde_json::from_value(message(other, "agent/fleet/fixture-example/cedar", "e"))
                    .unwrap();
            super::super::accept_message_receipt(&state, request, None, None).unwrap()
        };
        let (status, _) = call(
            &app,
            "GET",
            &format!("/v1/messages/read/{}", between_others.message.subject),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, own) = call(
            &app,
            "GET",
            &format!("/v1/messages/read/{}", sent["subject"].as_str().unwrap()),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{own}");

        // Everything else the daemon serves is refused.
        for (method, path) in [
            ("GET", "/v1/agents"),
            ("POST", "/v1/intent/apply"),
            ("POST", "/v1/work/ask"),
            ("GET", "/v1/documents"),
            ("GET", "/v1/client/terminals"),
            ("POST", "/v1/sekrets/attest"),
            ("GET", "/v1/status"),
        ] {
            let (status, _) = call(&app, method, path, Some(json!({}))).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        }
    }
}
