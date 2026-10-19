//! The part of `store` that moved up from st3-seats.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared_missions::{RUN_TREE_REBUILDS, STEP_TIMING_FOLD_CALLS, STEP_TIMING_ROWS_PARSED};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared_seats::{BASE_REBUILDS, DEFINITIONS_PARSED, RUN_PART_RUNS, STATUS_AFTER_DESIRED_READ, STEPS_ENRICHED, SUBJECT_REDUCTIONS};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared::*;
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
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use crate::model::{ApplyResponse, AttentionActionView, AttentionClosing, AttentionItemView, AttentionRequest, AttentionRequestView, AttentionResolveRequest, AttentionWithdrawRequest, Capability, ClaimInput, ClaimRecord, ClaimsPage, ClaimsSummary, ContextUsage, DependencySpec, DesiredSubject, EventRecord, FaultView, HumanReviewView, IntentInput, LoopRoundView, LoopRunView, MAX_EVAL_TIMEOUT_MS, SESSION_CLAIMS, MessageView, MissionDefinitionView, MissionInputKind, MissionResponse, MissionRevisionOperation, MissionRunDeclaration, MissionRunInput, MissionRunOutcomeView, MissionRunRequest, MissionRunView, MissionSpec, MissionState, NormalizedIntent, OperationalAnnotation, OperationalRepairItem, OperationalRepairPlan, OperationalRepairResult, PlannedAction, PlanningCandidateView, PlanningPreviewView, PlanningSessionDeclaration, PlanningSessionView, PlanningVariantView, ReplicaBatch, ReplicaRepairDeclaration, ResourceRefreshOperation, RevisionCutover, RevisionProposalView, RuntimeResetOperation, St3Error, StatusResponse, StepRunView, SubjectChange, SubjectStatus, UsageSummary, WorkSelector, WorkWakeView};
#[cfg(any(test, feature = "test-support"))]
use crate::model::WorkRequest;
#[cfg(test)]
use crate::model::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicationInventory, ReplicationReceipt};
#[cfg(any(test, feature = "test-support"))]
use crate::model::ReplicationExchange;
#[cfg(any(test, feature = "test-support"))]
#[cfg(test)]
#[cfg(any(test, feature = "test-support"))]
use smallclaims::replication::ReplicationBatch;
use crate::model::SeatQueueMoveView;
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
pub use st3_seats::store::runtime_starts::RuntimeStartOperation;
pub use attention_snapshot::{
    CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, native_prompt_gone_key,
    plan_approval, screen_prompt_delivered_key, screen_prompt_gone_key,
    screen_prompt_undelivered_key,
};
pub use st3_seats::store::mission_eligibility::MISSING_AGENT_CONDITION;
pub use st3_seats::store::database_size::DatabaseSize;
pub use st3_seats::store::checkpoint_rules::{RULES_VERSION, plan_drops, rules_digest};
pub use st3_seats::store::limits::{AccountLimit, LIMITS_CATCH_UP_PAGE};
pub use st3_seats::store::limits::ACCOUNT_READING_WINDOW_MS;
pub use st3_seats::store::subagents::{SUBAGENT_KINDS, SubagentView};
pub use st3_seats::store::runtime::SmalltalkRuntime;
#[cfg(any(test, feature = "test-support"))]
pub use smallclaims::sqlite::STATEMENTS_RUN;
use st3_kernel::model::LOCAL_OBSERVATION_ID_PREFIX;
pub use st3_kernel::model::local_observation_position;
pub use crate::incremental::{Change, ChangeFeed};
pub use crate::model::canonical_child_string;
pub use st3_seats::store::*;
pub use st3_seats::store::{ACTUAL_CACHE_LIMIT, ACTUAL_STATE_CLAIM, AgentStatusEntry, AgentWorkQueue, Aggregate, CachedUsageFold, ChildMissionContext, ClaimAdmission, ClaimCommitContext, ClaimsReduction, ClockFold, CumulativeUsage};
pub use st3_seats::store::{DesiredRow, EffectiveStepOwner, GATE_BROKEN_CONDITION, GRAPH_DIGEST_TABLES, HumanAttention, INCARNATION_OF_CLAIM, IdempotencyConflict, LOCAL_OBSERVATION_COLUMNS, MISSION_RUN_HEADER_SELECT, MessageCacheEntry, MissionRunEnrichment, OPEN_MISSION_RUN};
pub use st3_seats::store::{PinnedDefinitions, RANGE_SUBJECTS, RECENTLY_ENDED_MS, RUNTIME_ID_FIELD, RUNTIME_SUBJECTS, RecordedSeatMove, ReferenceGraph, RollupSpend, RosterStepRow, SeatQueueInputs, StatusEntry, StepLabel};
pub use st3_seats::store::{StepTiming, Store, SubjectCache, SubjectStatusInputs, UNREPAIRED_CLAIM, USAGE_BUCKETS, USAGE_RESPONSE_HORIZON_MS, UsageFold, UsageFolds, UsageSpend, ViewEntry, WORK_NUDGED_KIND};
pub use st3_seats::store::{accounts, active_step_blockers_tx, adhoc_work, agent_messages, agent_queue_values, analyze_mission_revision, append_claim_fenced_outcome, append_claim_tx, append_claim_with_commit_context, append_claim_with_fences, append_claim_with_subject_fences, append_latest_observation_publication};
pub use st3_seats::store::{append_receipt_claim_tx, apply_effective_step_state, apply_effective_step_state_with_owner, arm_waiting_mission_deadline_tx, arrangements, assistant_write, attention_ivm, backup, backup_format, cancel_descendant_mission_runs_tx, cancel_mission_run_tx, card_fold};
pub use st3_seats::store::{carried_claimants_tx, carried_revision_step_paths, carried_step_lease, carried_step_projection, check_mailbox_fence, check_mailbox_incarnation, check_observer_completion, checkpoint, checkpoint_rules, checkpoint_status, claim_append_error, claim_by_id_tx};
pub use st3_seats::store::{claim_log_order, classify_replicated_claim_with_registry, compatible_step_paths, conversation_reads, coordination, current_desired_row, current_desired_row_tx, current_harness_at, custom, database_size, declarations, delegation};
pub use st3_seats::store::{delivery, descendant_mission_run_ids_tx, desired_from_row, desired_lineage_from_tx, desired_revision, desired_row_at, directive_notes, dropped_claimant, enforce_mission_run_capacity, enrich_step_definition_cached, enrich_step_nudge_at, enrich_step_queue};
pub use st3_seats::store::{enrich_step_queue_at, enrich_step_queue_cached, enrich_step_summaries_at, enrich_step_wake_at, evidence, first_readiness_fault_tx, flatten_mission_step_specs, flatten_steps, fold_step_clock, fold_step_clock_open, fold_step_timing, fresh_context_ready_tx};
pub use st3_seats::store::{generation_id_from_subject, git_change_tables, github_workflow_failures, glass_heads, glasses, harness_fold_at, harness_questions, has_unknown_claim_at, idle_mail, insert_event, insert_local_observation_tx, intent_leaves_tx};
pub use st3_seats::store::{interpolate_goals, interpolate_selector, is_failed_terminal, is_terminal_generation_state, is_terminal_run_state, lanes, latest_actual, latest_actual_at, latest_claim_of_kind_tx, latest_retention, limits, listed_head_tx};
pub use st3_seats::store::{local_observation_dedupe_key, local_observation_from_row, local_retention, loop_run_views_tx, mailbox_changes, mailbox_lease, mailbox_wakes, mark_replay_safe_receipt_tx, message_bare_recipient, message_bodies, message_subscriptions, message_view_tx};
pub use st3_seats::store::{migrate_local_usage_seen, mission_definition_token_tx, mission_eligibility, mission_ivm, mission_list, mission_provenance, mission_run_header_from_row, mission_run_header_tx, mission_run_outcome_tx, mission_run_variables, mission_run_view_and_definition_tx, mission_run_view_for_projection_tx};
pub use st3_seats::store::{mission_run_view_for_reconcile_tx, mission_run_view_tx, mission_state_name, missions, newest_claims_of_kind_query, normalize_message_party, normalize_mission_run, normalize_resource_observation, normalize_seat, normalize_step_run, observer_actuals, observer_subscription_detail};
pub use st3_seats::store::{operation_audit, operational_repair_plan_tx, operations, owned_sets, person_work, pinned_generation_mission_tx, planning_session_view_tx, pricing_identity, project_claim_isolated_tx, project_mission_run_created, project_mission_run_update, project_replicated_base_claims};
pub use st3_seats::store::{project_replicated_mission_runs, project_revision_proposal, projection_digest, projection_divergence, projectors, publication, publication_facts, reapply_local_work_lease_renewals_tx, rebuild_planning_tx, rebuild_run_part_tx, rebuild_run_tree_tx, recently_ended_since};
pub use st3_seats::store::{reconcile_work_context, replay_graph_from_nothing_tx, resource_preparation, resources, retirements, revision_cutover_name, revision_proposal_view_tx, revision_seats, rollouts, roster_admission, run_part_of_tx, run_tree_of_tx};
pub use st3_seats::store::{runtime, runtime_list, runtime_starts, same_declared_launch, schema_fields_for_body, seat_queue_inputs_of_tx, seat_queue_inputs_tx, seat_queue_joins_query, seat_queue_moves_query, seat_status, seat_work_at_snapshot_query, seats};
pub use st3_seats::store::{select_replicated_desired, select_replicated_mission, selected_actual_source_at, sort_messages_canonically, stale_pull_request_request_tx, step_execution_clock_at, step_labels, step_owner_is_terminal_tx, step_run_from_row, step_run_row_tx, step_timeout_episode, step_timeout_extension_at};
pub use st3_seats::store::{step_timing_events, stop_deferral, subagents, subject_status_at, subscription_started_run_tx, system_local_retention, terminalize_generation_steps_tx, terminalize_projected_generation_steps_tx, terminalize_run_steps_tx, trace_principal, try_project_simple_replication_tx, unread_mail};
pub use st3_seats::store::{usage_native_session, usage_period, usage_slot, validate_actor, validate_claim_input, validate_local_observation, work_list, work_summaries_ivm};
pub mod agent_view;
pub mod attention;
pub mod attention_list;
pub mod attention_snapshot;
pub mod client_summary;
pub mod owner_lists;
pub mod published_list;
pub mod published_views;
pub mod summary_list;
#[cfg(test)]
pub mod checkpoint_agreement_tests;
#[cfg(test)]
pub mod checkpoint_capture_epoch_tests;
#[cfg(test)]
pub mod tests;

