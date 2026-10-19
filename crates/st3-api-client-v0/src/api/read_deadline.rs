//! The part of `api::read_deadline` that moved up from st3-api.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
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
use super::{ApiError, AppState, ClientTransportBoundary};
pub use st3_api_core::api::read_deadline::*;
pub use st3_api_core::api::read_deadline::{WorkError, envelope, error, handler, query, spawn_blocking, spawn_handler, spawn_mutation_handler, store_read};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use st3_api_core::api::read_deadline_tests_shared_client_v0_tests::{idle_long_poll_keeps_its_normal_empty_answer, request};

    #[test]
    fn deadlines_classify_reads_without_cancelling_writes_or_shortening_event_waits() {
        assert_eq!(
            deadline(&request(Method::GET, "/v1/mission-runs?root=r")),
            Some(ORDINARY)
        );
        assert_eq!(
            deadline(&request(Method::HEAD, "/v1/client/agents")),
            Some(ORDINARY)
        );
        assert_eq!(
            deadline(&request(Method::GET, "/v1/events?wait=true")),
            Some(EVENT)
        );
        assert_eq!(deadline(&request(Method::GET, "/v1/events")), Some(EVENT));
        assert_eq!(
            deadline(&request(
                Method::GET,
                "/v1/client/events?wait%5fms=%33%30%30%30%30"
            )),
            Some(EVENT)
        );
        assert_eq!(
            deadline(&request(Method::GET, "/v1/sessions/logs/seat?wait=%74rue")),
            Some(EVENT)
        );
        assert_eq!(
            deadline(&request(Method::GET, "/v1/events?wait=%66alse")),
            Some(ORDINARY)
        );
        assert_eq!(
            deadline(&request(Method::GET, "/v1/events?wait=false")),
            Some(ORDINARY)
        );
        assert_eq!(
            deadline(&request(Method::GET, "/v1/checkpoint/plan")),
            Some(BULK)
        );
        assert_eq!(
            deadline(&request(Method::POST, "/v1/internal/replication/export")),
            Some(BULK)
        );
        assert_eq!(deadline(&request(Method::POST, "/v1/messages/send")), None);
        assert_eq!(
            deadline(&request(Method::POST, "/v1/internal/replication/receive")),
            None
        );
    }

    #[test]
    fn synchronous_stream_queries_have_fresh_budgets_and_long_polls_have_query_children() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::api::tests_shared::state(root.path());
        assert!(read_budget::current().is_none());
        query(&state.store, "/stream/conversation", || {
            let current = read_budget::current().unwrap();
            assert_eq!(current.route(), "/stream/conversation");
            assert!(current.remaining() <= ORDINARY);
            assert!(state.store.readers.get().is_autocommit());
            Ok(())
        })
        .unwrap();
        assert!(read_budget::current().is_none());
        let poll = ReadBudget::new("/v1/client/events", EVENT);
        read_budget::with(Some(poll.clone()), || {
            query(&state.store, "/unused-fallback", || {
                let current = read_budget::current().unwrap();
                assert_eq!(current.route(), poll.route());
                assert!(current.remaining() <= ORDINARY);
                Ok(())
            })
            .unwrap();
            assert!(read_budget::current().unwrap().remaining() > ORDINARY);
        });
        assert!(read_budget::current().is_none());
    }

    #[test]
    fn synchronous_queries_refuse_expired_work_and_reject_late_success() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::api::tests_shared::state(root.path());
        let ran = AtomicUsize::new(0);
        let expired = ReadBudget::new("/v1/client/events", Duration::ZERO);
        let error = read_budget::with(Some(expired), || {
            query(&state.store, "/events", || {
                ran.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
        })
        .unwrap_err();
        assert_eq!(error.status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(ran.load(Ordering::Relaxed), 0);
        let parent = ReadBudget::new("/v1/client/events", ORDINARY);
        let error = read_budget::with(Some(parent.clone()), || {
            query(&state.store, "/events", || {
                parent.cancel();
                Ok(())
            })
        })
        .unwrap_err();
        assert_eq!(error.code, "read-deadline");
        assert!(state.store.readers.get().is_autocommit());
    }

    /// The one idle read-deadline test on the real clock: 30 real seconds, so the paused-clock
    /// tests below are not the only proof that the envelope really waits past the ordinary deadline.
    #[tokio::test]
    async fn an_idle_conversation_poll_is_not_cut_at_the_ordinary_deadline_on_the_real_clock() {
        idle_long_poll_keeps_its_normal_empty_answer(
            "/v1/client/conversations/test/changes?wait_ms=30000",
            "/v1/client/conversations/{id}/changes",
            false,
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_conversation_poll_is_not_cut_at_the_ordinary_deadline() {
        idle_long_poll_keeps_its_normal_empty_answer(
            "/v1/client/conversations/test/changes?wait_ms=30000",
            "/v1/client/conversations/{id}/changes",
            true,
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_client_event_poll_is_not_cut_at_the_ordinary_deadline() {
        idle_long_poll_keeps_its_normal_empty_answer(
            "/v1/client/events?wait_ms=30000",
            "/v1/client/events",
            true,
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_screen_poll_is_not_cut_at_the_ordinary_deadline() {
        idle_long_poll_keeps_its_normal_empty_answer(
            "/v1/client/terminals/test/screen?after=cursor&wait_ms=30000",
            "/v1/client/terminals/{id}/screen",
            true,
        )
        .await;
    }

    #[test]
    fn wait_budgets_follow_handler_defaults_caps_and_nonwaiting_requests() {
        for path in [
            "/v1/client/events",
            "/v1/client/conversations/test/changes",
            "/v1/client/terminals/test/screen?after=cursor",
            "/v1/sessions/logs/test?wait=false",
        ] {
            assert_eq!(deadline(&request(Method::GET, path)), Some(ORDINARY));
        }
        assert_eq!(
            deadline(&request(Method::GET, "/v1/client/events?wait_ms=999999")),
            Some(EVENT)
        );
        assert_eq!(
            deadline(&request(Method::GET, "/v1/events?timeout_ms=1234")),
            Some(ORDINARY + Duration::from_millis(1234))
        );
        assert_eq!(deadline(&request(Method::GET, "/v1/doctor")), Some(BULK));
    }
}
