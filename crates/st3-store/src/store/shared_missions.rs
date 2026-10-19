#![allow(unused_imports)]
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared_seats_shared_missions::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared_seats_shared_missions::{RUN_PART_RUNS, SUBJECT_REDUCTIONS};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, Transaction, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use crate::model::{ApplyResponse, AttentionActionView, AttentionClosing, AttentionItemView, AttentionRequest, AttentionRequestView, AttentionResolveRequest, AttentionWithdrawRequest, Capability, ClaimInput, ClaimRecord, ClaimsPage, ClaimsSummary, ContextUsage, DependencySpec, DesiredSubject, EventRecord, FaultView, HumanReviewView, IntentInput, LoopRoundView, LoopRunView, MAX_EVAL_TIMEOUT_MS, SESSION_CLAIMS, MessageView, MissionDefinitionView, MissionInputKind, MissionOutputView, MissionResponse, MissionRevisionOperation, MissionRunDeclaration, MissionRunInput, MissionRunOutcomeView, MissionRunReportView, MissionRunRequest, MissionRunView, MissionSpec, MissionState, NormalizedIntent, OperationalAnnotation, OperationalRepairItem, OperationalRepairPlan, OperationalRepairResult, PlannedAction, PlannerSpec, PlanningCandidateView, PlanningPreviewView, PlanningSessionDeclaration, PlanningSessionView, PlanningVariantView, ReplicaBatch, ReplicaRepairDeclaration, ResourceObservationOutcome, ResourceRefreshOperation, RevisionCutover, RevisionProposalView, RevisionSubmissionView, RunGenerationView, RuntimeResetOperation, St3Error, StatusResponse, StepRunView, SubjectChange, SubjectStatus, SubscriptionConditionSpec, SubscriptionRequestDecision, SubscriptionRequestView, SubscriptionSpec, UsageSummary, WorkRequest, WorkSelector, WorkWakeView};
use crate::model::{ReplicaEnvelope, ReplicaEnvelopeId, ReplicationInventory, ReplicationReceipt};
#[cfg(any(test, feature = "test-support"))]
use crate::model::ReplicationExchange;
#[cfg(any(test, feature = "test-support"))]
#[cfg(any(test, feature = "test-support"))]
use smallclaims::replication::ReplicationBatch;
use crate::model::{SeatQueueMoveRequest, SeatQueueMoveView};
use crate::seat_queue::{self, Placement, QueueJoin, QueueMove, SeatStep};
use smallclaims::claim::ReplicaEnvelopePayload;
use smallclaims::error::internal;
use smallclaims::hash::{batch_header_hash, canonical_hash, canonical_json_text, canonical_json_value, canonical_serialized_json_text, claim_hash};
#[cfg(any(test, feature = "test-support"))]
use smallclaims::hash::{claim_id_is_content_hash, verify_replica_batch_header};
use smallclaims::hash::{replica_envelope_hash, replica_record_ref};
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::Ordering;
#[allow(unused_imports)]
use smallclaims::sqlite::*;
pub use smallclaims::store::Store as GraphStore;
#[allow(unused_imports)]
use smallclaims::store::runtime::LegacyDigestTable;
#[allow(unused_imports)]
use smallclaims::store::*;
pub use smallclaims::store::{CHECKPOINT_ATTENTION_AFTER_MS, CHECKPOINT_EXCUSED, CHECKPOINT_MANIFEST_PAGE_LIMIT, CHECKPOINT_PROTOCOL, CHECKPOINT_SEALED, CHECKPOINT_VERIFIED, Certificate, CheckpointAction, CheckpointClaim, CheckpointContext, CheckpointExcuseRequest, CheckpointManifest, CheckpointManifestCursor, CheckpointManifestNeed, CheckpointManifestPage, CheckpointManifestRequest, CheckpointPlanRequest, CheckpointPlanView, CheckpointProof, CheckpointResumeRequest, CheckpointStatusView, ClaimTombstone, CreatedFleetInvite, DropCount, DropPlan, EnvelopeKey, EnvelopeTombstone, FleetAdmissionResidue, FleetInviteView, FleetRedemption, FleetRemoval, PendingCheckpointView, ReplicationAdmission, SealTerms, SealedSet, TRIM_CHUNK_ENVELOPES, TrimFault, VerifiedTerms, certificates, checkpoint_build, checkpoint_cut, checkpoint_name, checkpoint_participants, chosen_certificate, configure_projection_writer, drop_digest, excused_writers, first_verifications, newest_due_cut, newest_seals, stable_checkpoints, valid_fleet_node_name, verify_checkpoint_manifest};
pub use crate::store::runtime_starts::RuntimeStartOperation;
pub use crate::store::attention_snapshot::{CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, native_prompt_gone_key, plan_approval, screen_prompt_delivered_key, screen_prompt_gone_key, screen_prompt_undelivered_key};
pub use crate::store::mission_eligibility::MISSING_AGENT_CONDITION;
pub use crate::store::database_size::DatabaseSize;
pub use crate::store::checkpoint_rules::{RULES_VERSION, plan_drops, rules_digest};
pub use crate::store::limits::{AccountLimit, LIMITS_CATCH_UP_PAGE};
pub use crate::store::limits::ACCOUNT_READING_WINDOW_MS;
pub use crate::store::subagents::{SUBAGENT_KINDS, SubagentView};
pub use crate::store::runtime::SmalltalkRuntime;
#[cfg(any(test, feature = "test-support"))]
pub use smallclaims::sqlite::STATEMENTS_RUN;
use crate::resource::newer_comment;
use st3_kernel::model::LOCAL_OBSERVATION_ID_PREFIX;
pub use st3_kernel::model::local_observation_position;
pub use crate::incremental::{Change, ChangeFeed};
pub use crate::model::canonical_child_string;
use crate::store::*;
use crate::store::{ACTUAL_CACHE_LIMIT, ACTUAL_STATE_CLAIM, AgentStatusEntry, AgentWorkQueue, Aggregate, CachedUsageFold, ChildMissionContext, ClaimAdmission, ClaimCommitContext, ClaimsReduction, ClockFold, CumulativeUsage};
use crate::store::{DesiredRow, EffectiveStepOwner, GATE_BROKEN_CONDITION, GRAPH_DIGEST_TABLES, HumanAttention, INCARNATION_OF_CLAIM, IdempotencyConflict, LOCAL_OBSERVATION_COLUMNS, MISSION_RUN_HEADER_SELECT, MailboxWatermark, MessageCacheEntry, MissionRunEnrichment};
use crate::store::{OPEN_MISSION_RUN, PinnedDefinitions, RANGE_SUBJECTS, RECENTLY_ENDED_MS, RUNTIME_ID_FIELD, RUNTIME_SUBJECTS, RecordedSeatMove, ReferenceGraph, RollupSpend, RosterStepRow, SeatQueueInputs, StatusEntry};
use crate::store::{StepLabel, StepTiming, Store, SubjectCache, SubjectStatusInputs, UNREPAIRED_CLAIM, USAGE_BUCKETS, USAGE_RESPONSE_HORIZON_MS, UsageFold, UsageFolds, UsageSpend, ViewEntry};
use crate::store::{WORK_NUDGED_KIND, active_step_blockers_tx, analyze_mission_revision, append_claim_fenced_outcome, append_claim_tx, append_claim_with_commit_context, append_claim_with_fences, append_claim_with_subject_fences, append_latest_observation_publication, append_receipt_claim_tx, apply_effective_step_state, apply_effective_step_state_with_owner};
use crate::store::{arm_waiting_mission_deadline_tx, cancel_descendant_mission_runs_tx, cancel_mission_run_tx, carried_claimants_tx, carried_revision_step_paths, carried_step_lease, carried_step_projection, check_mailbox_fence, check_observer_completion, claim_append_error, claim_by_id_tx, claim_log_order};
use crate::store::{classify_replicated_claim_with_registry, compatible_step_paths, current_desired_row, current_desired_row_tx, current_harness_at, descendant_mission_run_ids_tx, desired_from_row, desired_lineage_from_tx, desired_revision, desired_row_at, dropped_claimant, enforce_mission_run_capacity};
use crate::store::{enrich_step_definition_cached, enrich_step_nudge_at, enrich_step_queue, enrich_step_queue_at, enrich_step_queue_cached, enrich_step_summaries_at, first_readiness_fault_tx, flatten_mission_step_specs, flatten_steps, fold_step_clock, fold_step_clock_open, fold_step_timing};
use crate::store::{fresh_context_ready_tx, generation_id_from_subject, harness_fold_at, has_unknown_claim_at, insert_local_observation_tx, intent_leaves_tx, interpolate_goals, interpolate_selector, is_failed_terminal, is_terminal_generation_state, is_terminal_run_state, latest_actual};
use crate::store::{latest_actual_at, latest_claim_of_kind_tx, latest_retention, listed_head_tx, local_observation_dedupe_key, local_observation_from_row, local_retention, loop_run_views_tx, mark_replay_safe_receipt_tx, migrate_local_usage_seen, mission_definition_token_tx, mission_run_header_from_row};
use crate::store::{mission_run_header_tx, mission_run_outcome_tx, mission_run_variables, mission_run_view_and_definition_tx, mission_run_view_for_projection_tx, mission_run_view_for_reconcile_tx, mission_run_view_tx, mission_state_name, newest_claims_of_kind_query, normalize_message_party, normalize_mission_run, normalize_resource_observation};
use crate::store::{normalize_seat, normalize_step_run, operational_repair_plan_tx, pinned_generation_mission_tx, planning_session_view_tx, pricing_identity, project_claim_isolated_tx, project_mission_run_created, project_mission_run_update, project_replicated_base_claims, project_replicated_mission_runs, project_revision_proposal};
use crate::store::{reapply_local_work_lease_renewals_tx, rebuild_planning_tx, rebuild_run_part_tx, rebuild_run_tree_tx, recently_ended_since, replay_graph_from_nothing_tx, revision_cutover_name, revision_proposal_view_tx, run_part_of_tx, run_tree_of_tx, runtime, runtime_only_authority};
use crate::store::{same_declared_launch, schema_fields_for_body, seat_queue_inputs_of_tx, seat_queue_inputs_tx, seat_queue_joins_query, seat_queue_moves_query, seat_work_at_snapshot_query, select_replicated_desired, select_replicated_mission, selected_actual_source_at, stale_pull_request_request_tx, step_execution_clock_at};
use crate::store::{step_owner_is_terminal_tx, step_run_from_row, step_run_row_tx, step_timeout_episode, step_timeout_extension_at, step_timing_events, subject_status_at, subscription_started_run_tx, system_local_retention, terminalize_generation_steps_tx, terminalize_projected_generation_steps_tx, terminalize_run_steps_tx};
use crate::store::{try_project_simple_replication_tx, usage_native_session, usage_slot, validate_claim_input, validate_local_observation};
#[cfg(any(test, feature = "test-support"))]
use crate::store::{WORK_ACTION_WRITER_WORK, WorkActionWriterProbe, append_latest_observation, expire_work_lease_by_claim, mailbox_fence_evaluations, mailbox_incarnation_evaluations, message_canonical_key_query, message_canonical_key_tx, named_run_probes, runtime_view_entry, step_execution_timing_at, try_project_all_simple_replication_tx};
#[cfg(any(test, feature = "test-support"))]
use crate::store::{validate_replicated_claim, verify_replica_batch};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Run trees this thread rebuilt from their own claims.
    pub static RUN_TREE_REBUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static STEP_TIMING_FOLD_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub static STEP_TIMING_ROWS_PARSED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
