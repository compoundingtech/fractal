//! The part of `api` that moved up from st3-api.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::AttentionSnapshotStore;
use crate::AttentionStore;
use crate::ConversationReadsStore;
use crate::DeliveryStore;
use crate::DeliveryStoreCore;
use crate::MailboxLeaseStore;
use crate::AdhocWorkStore;
use crate::CoordinationStore;
use crate::DelegationStore;
use crate::MissionsStore;
use crate::MissionsStoreCore;
use crate::WorkListStore;
use crate::LimitsStore;
use crate::RosterAdmissionStore;
use crate::SeatStatusStore;
use crate::SeatsStore;
use crate::SeatsStoreCore;
use crate::SubagentsStore;
use crate::EvidenceStore;
use crate::OwnedSetsStore;
use crate::PublicationFactsStore;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use axum::body::{Body, to_bytes};
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Extension, Path as AxumPath, Query, State};
use axum::http::{Request, StatusCode};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use futures_util::{SinkExt as _, StreamExt as _};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::net::TcpListener;
use tokio::sync::{Notify, watch};
use tower::ServiceExt as _;
use crate::archive::hydrate_eval;
use crate::graph::{parse_intent, resolve_document_references};
use crate::message_body;
#[cfg(test)]
use crate::model::AttentionRequest;
pub use crate::client_values::{
    client_attention_actions, client_attention_id, client_timestamp, client_work_state,
};
pub use crate::store::agent_queue_values::{
    AGENT_QUEUE_FIELDS, agent_queue_fields, agent_queue_metadata,
};
use crate::model::ClientReplicated;
use crate::model::{
    ApplyRequest, ApplyResponse, AttachRequest, Attachment, AttentionItemView,
    AttentionRequestView, AttentionResolveRequest, AttentionWithdrawRequest, ClaimInput,
    ClaimRecord, ClaimsPage, ClientPageInfo, ClientResourcePage, ClientSyncNotice, ClientSyncPeer,
    ContextClearRequest, DoctorCheck, DoctorReport, DocumentListResponse, DocumentPutRequest,
    DocumentVersion, EvalStartRequest, EvalStartResponse, EvalStatus, EventRecord,
    GateResultRequest, HumanReviewView, IntentInput, LaunchApproveAndStartRequest,
    LaunchApproveAndStartView, LaunchDecisionAnswerRequest, LaunchDecisionOption,
    LaunchDecisionRequest, LaunchDecisionResponse, LaunchDecisionType, LaunchStartRequest,
    LocalTerminal, MAX_EVAL_TIMEOUT_MS, MessageLifecycleRequest, MessagePage, MessageSendReceipt,
    MessageSendRequest, MessageView, MissionOutputView, MissionProductionRequest, MissionRequest,
    MissionResponse, MissionRetireRequest, MissionRevisionRequest, MissionRunOutcomeRequest,
    MissionRunReportRequest, MissionRunReportView, MissionRunRequest, MissionRunView,
    OperationalRepairApplyRequest, OperationalRepairPlan, OperationalRepairResult, PlannerSpec,
    PlanningApprovalRequest, PlanningCancelRequest, PlanningCandidateSubmitRequest,
    PlanningProposalRequest, PlanningRevisionRequest,
    PlanningSessionStartRequest, PlanningSessionView, QuickAgentRequest, QuickAgentResponse,
    ReplicaRecordView, ReplicationExportRequest, ReplicationExportResponse,
    ReplicationPeerFailureRequest, ReplicationReceiveRequest, ReplicationReceiveResponse,
    ReplicationRepairRequest, ReplicationStatus, ReviewRequest, RevisionApprovalRequest,
    RevisionCancelRequest, RevisionCutover, RevisionProposalView, RevisionSubmissionView,
    RunGenerationView, SessionControlResponse, SessionInputMode, SessionInputRequest,
    SessionLogChunk, SessionScreen, SessionSignalRequest, St3Error, StatusResponse, StepRunView,
    WorkRequest, WorkRetryRequest, WorkWakeRequest,
};
use crate::model::{PersonAskRequest, PersonStepResponse};
use crate::store::Store;
use crate::delivery_presence;
pub use st3_api::api::published_attention::start_attention_list;
#[cfg(any(test, feature = "test-support"))]
pub use st3_api::api::published_attention::{published_attention_rows, refresh_attention_list};
#[cfg(target_os = "linux")]
pub use tile_endpoint::{
    SOCKET as TILE_ENDPOINT_SOCKET, spawn as spawn_tile_endpoints,
    spawn_from as spawn_tile_endpoints_from,
};
pub use published_lists::{start_published_lists, wait_published_lists_at_least};
// Other api code calls these handlers directly.
use st3_api::api::launches::{
    approve_planning_session, cancel_planning_session, mission, preview_named_planning_candidate,
    revise_planning_session, start_planning_session,
};
use st3_api::api::messages::post_message_claim;
use st3_api::api::operations::{get_slo, get_slo_history};
use st3_api::api::records::post_review;
use st3_api::api::sessions::clear_context;
use st3_api::api::work::retry_work;
// The tests call these handlers directly.
#[cfg(test)]
use st3_api::api::client_reads::{
    client_agents, client_agents_detail, client_history, client_history_detail,
    client_launches_detail, client_work,
};
#[cfg(test)]
use st3_api::api::operations::events;
#[cfg(test)]
use st3_api::api::records::post_claim;
#[cfg(test)]
use st3_api::api::replication::{replication_export, replication_wake};
pub use client_v0::raw_terminal::splice as raw_terminal_splice;
pub use crate::delivery_presence::claude_channel_attached;
pub use crate::peer::track_remote_reads;
pub use crate::model::new_request_id;
pub use crate::model::FleetEndpointsRequest;
pub use st3_api::api::*;
pub use st3_api::api::{AUTHORITY_INDETERMINATE_REASON, AgentSuspensionRequest, ApiError, AppState, BoundAgent, ClientListQuery, ClientPageResponse, ClientSnapshot, ClientTransportBoundary, DEVICE_SIGNATURE_WINDOW_MS, EventQuery, FleetInviteCreated};
pub use st3_api::api::{FleetInviteRequest, FleetInviteRevokeRequest, FleetPersonRequest, FleetRemoveRequest, FleetStatus, LiveSession, LocalPeer, MessageRead, MessagesQuery, NativeDeliveryPeer, PUBLISH_SAMPLE_CLAIMS, ReservedUnixSocket};
pub use st3_api::api::{RouteFamily, RuleAudit, RuleSetRequest, SIGNED_LONG_MESSAGE_FIELDS, SIGNED_MESSAGE_FIELDS, SloHistoryQuery, SuspensionFence, WorkQuery, agent_harness, apply, blocking_action, client_attention_resources_at};
pub use st3_api::api::{client_now_ms, client_reads, cold_agent_roster_rebuild, fabric_router_with, harness_events, is_work_wake, launches, mail_backlog, mailbox, messages, mission_eligibility_doctor};
pub use st3_api::api::{operations, published_attention, raw_terminal_owner_splice, record_publish_latency, record_stream_latency, records, replication, request_latency_target_windows, request_latency_windows, request_resume, request_suspend, reserve_unix_socket};
pub use st3_api::api::{response_envelope, router_with, serve_reserved_unix_with_ready, serve_tcp, serve_unix, serve_unix_bound, serve_unix_bound_with_ready, serve_unix_with_ready, sessions, signal_changed, signal_local_change};
pub use st3_api::api::{signal_message_changed, signal_visible_change, slo_rows, start_agent_roster, start_native_session_discovery, start_operation_report, unix_reservation_path, work};
pub mod client_v0;

/// This crate's route family. st3-daemon serves it beside every other family (its `api::router`);
/// this crate's router is st3-api's routes and the client-v0 routes, for its own tests.
const ROUTE_FAMILIES: &[RouteFamily] = &[client_v0::routes];

/// Build routes only. The owner starts background publications with
/// [`start_published_lists`]; a bare router refuses unpublished current runtime reads.
pub fn router(state: AppState) -> Router {
    router_with(state, ROUTE_FAMILIES)
}

/// Build the loopback-only client gateway. Unlike the local Unix boundary, every ordinary
/// client request on this router requires a paired bearer credential.
pub fn fabric_router(state: AppState) -> Router {
    fabric_router_with(state, ROUTE_FAMILIES)
}
pub mod published_lists;
pub mod read_deadline;
#[cfg(target_os = "linux")]
pub mod tile_endpoint;
#[cfg(test)]
mod tests;

