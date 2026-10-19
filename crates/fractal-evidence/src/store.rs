//! The part of `store` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
#[cfg(test)]
use std::fs;
use std::ops::Deref;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use crate::model::{
    ApplyResponse, AttentionActionView, AttentionClosing, AttentionItemView, AttentionRequest,
    AttentionRequestView, AttentionResolveRequest, AttentionWithdrawRequest, Capability,
    ClaimInput, ClaimRecord, ClaimsPage, ClaimsSummary, ContextUsage, DependencySpec,
    DesiredSubject, EventRecord, FaultView, HumanReviewView, IntentInput, LoopRoundView,
    LoopRunView, MAX_EVAL_TIMEOUT_MS, SESSION_CLAIMS, MessageView, MissionDefinitionView,
    MissionInputKind, MissionOutputView, MissionResponse, MissionRevisionOperation,
    MissionRunDeclaration, MissionRunInput, MissionRunOutcomeView, MissionRunReportView,
    MissionRunRequest, MissionRunView, MissionSpec, MissionState, NormalizedIntent,
    OperationalAnnotation, OperationalRepairItem, OperationalRepairPlan, OperationalRepairResult,
    PlannedAction, PlannerSpec, PlanningCandidateView, PlanningPreviewView,
    PlanningSessionDeclaration, PlanningSessionView, PlanningVariantView, ReplicaBatch,
    ReplicaRepairDeclaration, ResourceObservationOutcome, ResourceRefreshOperation,
    RevisionCutover, RevisionProposalView, RevisionSubmissionView, RunGenerationView,
    RuntimeResetOperation, St3Error, StatusResponse, StepRunView, SubjectChange, SubjectStatus,
    SubscriptionConditionSpec, SubscriptionRequestDecision, SubscriptionRequestView,
    SubscriptionSpec, UsageSummary, WorkRequest, WorkSelector, WorkWakeView,
};
#[cfg(test)]
use crate::model::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicationInventory, ReplicationReceipt};
#[cfg(any(test, feature = "test-support"))]
use crate::model::ReplicationExchange;
#[cfg(any(test, feature = "test-support"))]
use smallclaims::replication::{ReplicaRange, ReplicationBatch, ReplicationResponse};
use crate::model::{SeatQueueMoveRequest, SeatQueueMoveView, SeatQueueRunView, SeatQueueView};
use crate::seat_queue::{self, Placement, QueueJoin, QueueMove, SeatStep};
#[cfg(test)]
use smallclaims::claim::ReplicaEnvelopePayload;
use smallclaims::error::internal;
use smallclaims::hash::{
    batch_header_hash, canonical_hash, canonical_json_text, canonical_json_value,
    canonical_serialized_json_text, claim_hash,
};
#[cfg(any(test, feature = "test-support"))]
use smallclaims::hash::{claim_id_is_content_hash, verify_replica_batch_header};
#[cfg(test)]
use smallclaims::hash::{replica_envelope_hash, replica_record_ref};
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::Ordering;
// The graph half of the store: the claim log, envelopes, replication, checkpoints,
// documents and blobs. Everything it exports is in scope here as it was before the carve.
#[allow(unused_imports)]
use smallclaims::sqlite::*;
pub use smallclaims::store::Store as GraphStore;
#[allow(unused_imports)]
use smallclaims::store::runtime::LegacyDigestTable;
#[allow(unused_imports)]
use smallclaims::store::*;
pub use smallclaims::store::{
    CHECKPOINT_ATTENTION_AFTER_MS, CHECKPOINT_EXCUSED, CHECKPOINT_MANIFEST_PAGE_LIMIT,
    CHECKPOINT_PROTOCOL, CHECKPOINT_SEALED, CHECKPOINT_VERIFIED, Certificate, CheckpointAction,
    CheckpointClaim, CheckpointContext, CheckpointExcuseRequest, CheckpointManifest,
    CheckpointManifestCursor, CheckpointManifestNeed, CheckpointManifestPage,
    CheckpointManifestRequest, CheckpointPlanRequest, CheckpointPlanView, CheckpointProof,
    CheckpointResumeRequest, CheckpointStatusView, ClaimTombstone, CreatedFleetInvite, DropCount,
    DropPlan, EnvelopeKey, EnvelopeTombstone, FleetAdmissionResidue, FleetInviteView,
    FleetRedemption, FleetRemoval, PendingCheckpointView, ReplicationAdmission, SealTerms,
    SealedSet, TRIM_CHUNK_ENVELOPES, TrimFault, VerifiedTerms, certificates, checkpoint_build,
    checkpoint_cut, checkpoint_name, checkpoint_participants, chosen_certificate,
    configure_projection_writer, drop_digest, excused_writers, first_verifications, newest_due_cut,
    newest_seals, stable_checkpoints, valid_fleet_node_name, verify_checkpoint_manifest,
};
pub use fractal_attention::store::runtime_starts::{RuntimeStartAdmission, RuntimeStartOperation, RuntimeStartLock};
pub use fractal_attention::store::roster_admission::AdmissionGuard;
pub use fractal_attention::store::attention_snapshot::{
    CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, native_prompt_gone_key,
    plan_approval, screen_prompt_delivered_key, screen_prompt_gone_key,
    screen_prompt_undelivered_key,
};
pub use fractal_attention::store::mission_eligibility::MISSING_AGENT_CONDITION;
pub use fractal_attention::store::database_size::DatabaseSize;
pub use fractal_attention::store::person_work::AskImages;
pub use fractal_attention::store::checkpoint_rules::{RULES_VERSION, plan_drops, rules_digest};
pub use fractal_attention::store::limits::{AccountLimit, LIMITS_ACTOR, LIMITS_CATCH_UP_PAGE, LimitsOutcome, LimitsPolicy};
pub use fractal_attention::store::limits::ACCOUNT_READING_WINDOW_MS;
pub use fractal_attention::store::subagents::{
    EndedSubagent, SUBAGENT_KINDS, SUBAGENT_LEASE_MS, SubagentSweep, SubagentView,
};
pub use runtime::SmalltalkRuntime;
#[cfg(any(test, feature = "test-support"))]
pub use smallclaims::sqlite::STATEMENTS_RUN;
use crate::resource::newer_comment;
use fractal_kernel::model::LOCAL_OBSERVATION_ID_PREFIX;
pub use fractal_kernel::model::local_observation_position;
pub use crate::incremental::{Change, ChangeFeed};
pub use crate::model::canonical_child_string;
pub use fractal_attention::store::*;
pub use fractal_attention::store::{AgentResourcesChange, AgentResourcesDelta, AgentWorkQueue, CachedUsageFold, ChildMissionContext, ClaimAdmission, ClaimCommitContext, ClaimsReduction, DesiredRow, DiscoveredItem, GATE_BROKEN_CONDITION, HumanAttention};
pub use fractal_attention::store::{IdempotencyConflict, MAX_STEP_WAIT_MS, MailboxWatermark, MessageCacheEntry, RANGE_SUBJECTS, RECENTLY_ENDED_MS, RUN_REPORT_KIND, StepLabel, StepTiming, Store, SubjectCache, USAGE_RESPONSE_HORIZON_MS};
pub use fractal_attention::store::{UsageFold, UsageFolds, ViewEntry, WORK_NUDGED_KIND, accounts, adhoc_work, agent_messages, agent_queue_values, agent_view, analyze_mission_revision, append_claim_fenced_outcome, append_claim_tx};
pub use fractal_attention::store::{append_claim_with_commit_context, arrangements, assistant_write, attention, attention_snapshot, backup, backup_format, card_fold, check_mailbox_fence, checkpoint, checkpoint_rules, checkpoint_status};
pub use fractal_attention::store::{claim_append_error, claim_log_order, client_summary, conversation_reads, coordination, custom, database_size, declarations, delegation, delivery, desired_from_row, directive_notes};
pub use fractal_attention::store::{end_watch_tx, fold_step_clock, fold_step_timing, format_wait_span, generation_id_from_subject, git_change_tables, github_post_agent_tx, github_workflow_failures, glass_heads, glasses, harness_questions, idle_mail};
pub use fractal_attention::store::{lanes, limits, loop_run_views_tx, mailbox_changes, mailbox_lease, mailbox_wakes, message_bodies, message_subscriptions, migrate_local_usage_seen, mission_eligibility, mission_ivm, mission_list};
pub use fractal_attention::store::{mission_provenance, mission_run_header_tx, mission_run_outcome_tx, mission_run_variables, missions, normalize_message_party, observer_actuals, observer_subscription_detail, operation_audit, operations, owner_lists, person_work};
pub use fractal_attention::store::{projection_digest, projection_divergence, projectors, published_list, recently_ended_since, reconcile_work_context, replay_graph_from_nothing_tx, resource_preparation, resources, retirements, revision_seats, rollouts};
pub use fractal_attention::store::{roster_admission, run_report_fields, runtime_list, runtime_starts, seat_queue_joins_query, seat_queue_moves_query, seat_status, seats, stale_pull_request_request_tx, step_execution_clock_at, step_labels, step_run_from_row};
pub use fractal_attention::store::{step_timeout_episode, step_timeout_extension_at, step_timing_lease_at, stop_deferral, subagents, subscription_started_run_tx, summary_list, trace_principal, unread_mail, usage_period, validate_claim_input, watch_ended_tx};
pub use fractal_attention::store::{work_list, work_summaries_ivm};
pub mod attention_list;
pub mod evidence;
pub mod owned_sets;
pub mod publication;
pub mod publication_facts;
pub mod published_views;
pub mod runtime;
#[cfg(test)]
pub mod owned_sets_tests;
#[cfg(test)]
pub mod roster_fresh_proof_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod terminal_gate_evidence;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionGateRunner {
    pub subject: String,
    pub host: String,
    pub owner_run: String,
    pub owner_step: Option<String>,
    pub retired: bool,
}
