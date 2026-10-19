//! The part of `store` that moved up from st3-store.
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
use crate::model::{ApplyResponse, AttentionActionView, AttentionClosing, AttentionItemView, AttentionRequest, AttentionRequestView, AttentionResolveRequest, AttentionWithdrawRequest, Capability, ClaimInput, ClaimRecord, ClaimsPage, ClaimsSummary, ContextUsage, DependencySpec, DesiredSubject, EventRecord, FaultView, HumanReviewView, IntentInput, LoopRoundView, LoopRunView, MAX_EVAL_TIMEOUT_MS, SESSION_CLAIMS, MessageView, MissionDefinitionView, MissionInputKind, MissionResponse, MissionRevisionOperation, MissionRunDeclaration, MissionRunInput, MissionRunOutcomeView, MissionRunRequest, MissionRunView, MissionSpec, MissionState, NormalizedIntent, OperationalAnnotation, OperationalRepairItem, OperationalRepairPlan, OperationalRepairResult, PlannedAction, PlanningCandidateView, PlanningPreviewView, PlanningSessionDeclaration, PlanningSessionView, PlanningVariantView, ReplicaBatch, ReplicaRepairDeclaration, ResourceRefreshOperation, RevisionCutover, RevisionProposalView, RuntimeResetOperation, St3Error, StatusResponse, StepRunView, SubjectChange, SubjectStatus, SubscriptionRequestDecision, SubscriptionRequestView, UsageSummary, WorkRequest, WorkSelector, WorkWakeView};
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
pub use st3_store::store::runtime_starts::RuntimeStartOperation;
pub use st3_store::store::attention_snapshot::{
    CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, native_prompt_gone_key,
    plan_approval, screen_prompt_delivered_key, screen_prompt_gone_key,
    screen_prompt_undelivered_key,
};
pub use st3_store::store::mission_eligibility::MISSING_AGENT_CONDITION;
pub use st3_store::store::database_size::DatabaseSize;
pub use st3_store::store::checkpoint_rules::{RULES_VERSION, plan_drops, rules_digest};
pub use st3_store::store::limits::{AccountLimit, LIMITS_CATCH_UP_PAGE};
pub use st3_store::store::limits::ACCOUNT_READING_WINDOW_MS;
pub use st3_store::store::subagents::{SUBAGENT_KINDS, SubagentView};
pub use st3_store::store::runtime::SmalltalkRuntime;
#[cfg(any(test, feature = "test-support"))]
pub use smallclaims::sqlite::STATEMENTS_RUN;
use st3_kernel::model::LOCAL_OBSERVATION_ID_PREFIX;
pub use st3_kernel::model::local_observation_position;
pub use crate::incremental::{Change, ChangeFeed};
pub use crate::model::canonical_child_string;
pub use st3_store::store::*;
pub use st3_store::store::{ACTUAL_CACHE_LIMIT, ACTUAL_STATE_CLAIM, AgentStatusEntry, AgentWorkQueue, Aggregate, CachedUsageFold, ChildMissionContext, ClaimAdmission, ClaimCommitContext, ClaimsReduction, ClockFold, CumulativeUsage};
pub use st3_store::store::{DesiredRow, EffectiveStepOwner, GATE_BROKEN_CONDITION, GRAPH_DIGEST_TABLES, HumanAttention, INCARNATION_OF_CLAIM, IdempotencyConflict, LOCAL_OBSERVATION_COLUMNS, MISSION_RUN_HEADER_SELECT, MessageCacheEntry, MissionRunEnrichment, OPEN_MISSION_RUN};
pub use st3_store::store::{PinnedDefinitions, RANGE_SUBJECTS, RECENTLY_ENDED_MS, RUNTIME_ID_FIELD, RUNTIME_SUBJECTS, RecordedSeatMove, ReferenceGraph, RollupSpend, RosterStepRow, SeatQueueInputs, StatusEntry, StepLabel};
pub use st3_store::store::{StepTiming, Store, SubjectCache, SubjectStatusInputs, UNREPAIRED_CLAIM, USAGE_BUCKETS, USAGE_RESPONSE_HORIZON_MS, UsageFold, UsageFolds, UsageSpend, ViewEntry, WORK_NUDGED_KIND};
pub use st3_store::store::{accounts, active_step_blockers_tx, adhoc_work, agent_view, analyze_mission_revision, append_claim_fenced_outcome, append_claim_tx, append_claim_with_commit_context, append_claim_with_fences, append_claim_with_subject_fences, append_latest_observation_publication, append_receipt_claim_tx};
pub use st3_store::store::{apply_effective_step_state, apply_effective_step_state_with_owner, arm_waiting_mission_deadline_tx, arrangements, attention, attention_list, attention_snapshot, backup, backup_format, cancel_descendant_mission_runs_tx, cancel_mission_run_tx, card_fold};
pub use st3_store::store::{carried_claimants_tx, carried_revision_step_paths, carried_step_lease, carried_step_projection, check_mailbox_fence, check_observer_completion, checkpoint, checkpoint_rules, checkpoint_status, claim_append_error, claim_by_id_tx, claim_log_order};
pub use st3_store::store::{classify_replicated_claim_with_registry, client_summary, compatible_step_paths, current_desired_row, current_desired_row_tx, current_harness_at, database_size, declarations, delegation, descendant_mission_run_ids_tx, desired_from_row, desired_lineage_from_tx};
pub use st3_store::store::{desired_revision, desired_row_at, dropped_claimant, enforce_mission_run_capacity, enrich_step_definition_cached, enrich_step_nudge_at, enrich_step_queue, enrich_step_queue_at, enrich_step_queue_cached, enrich_step_summaries_at, evidence, first_readiness_fault_tx};
pub use st3_store::store::{flatten_mission_step_specs, flatten_steps, fold_step_clock, fold_step_clock_open, fold_step_timing, fresh_context_ready_tx, generation_id_from_subject, git_change_tables, github_workflow_failures, glass_heads, harness_fold_at, harness_questions};
pub use st3_store::store::{has_unknown_claim_at, idle_mail, insert_local_observation_tx, intent_leaves_tx, interpolate_goals, interpolate_selector, is_failed_terminal, is_terminal_generation_state, is_terminal_run_state, lanes, latest_actual, latest_actual_at};
pub use st3_store::store::{latest_claim_of_kind_tx, latest_retention, limits, listed_head_tx, local_observation_dedupe_key, local_observation_from_row, local_retention, loop_run_views_tx, mark_replay_safe_receipt_tx, message_subscriptions, migrate_local_usage_seen, mission_definition_token_tx};
pub use st3_store::store::{mission_eligibility, mission_ivm, mission_list, mission_provenance, mission_run_header_from_row, mission_run_header_tx, mission_run_outcome_tx, mission_run_variables, mission_run_view_and_definition_tx, mission_run_view_for_projection_tx, mission_run_view_for_reconcile_tx, mission_run_view_tx};
pub use st3_store::store::{mission_state_name, missions, newest_claims_of_kind_query, normalize_message_party, normalize_mission_run, normalize_resource_observation, normalize_seat, normalize_step_run, observer_actuals, observer_subscription_detail, operation_audit, operational_repair_plan_tx};
pub use st3_store::store::{operations, owned_sets, owner_lists, person_work, pinned_generation_mission_tx, planning_session_view_tx, pricing_identity, project_claim_isolated_tx, project_mission_run_created, project_mission_run_update, project_replicated_base_claims, project_replicated_mission_runs};
pub use st3_store::store::{project_revision_proposal, projection_digest, projectors, publication, publication_facts, published_list, published_views, reapply_local_work_lease_renewals_tx, rebuild_planning_tx, rebuild_run_part_tx, rebuild_run_tree_tx, recently_ended_since};
pub use st3_store::store::{reconcile_work_context, replay_graph_from_nothing_tx, resource_preparation, resources, retirements, revision_cutover_name, revision_proposal_view_tx, revision_seats, rollouts, roster_admission, run_part_of_tx, run_tree_of_tx};
pub use st3_store::store::{runtime, runtime_list, runtime_starts, same_declared_launch, schema_fields_for_body, seat_queue_inputs_of_tx, seat_queue_inputs_tx, seat_queue_joins_query, seat_queue_moves_query, seat_status, seat_work_at_snapshot_query, seats};
pub use st3_store::store::{select_replicated_desired, select_replicated_mission, selected_actual_source_at, stale_pull_request_request_tx, step_execution_clock_at, step_labels, step_owner_is_terminal_tx, step_run_from_row, step_run_row_tx, step_timeout_episode, step_timeout_extension_at, step_timing_events};
pub use st3_store::store::{stop_deferral, subagents, subject_status_at, subscription_started_run_tx, summary_list, system_local_retention, terminalize_generation_steps_tx, terminalize_projected_generation_steps_tx, terminalize_run_steps_tx, trace_principal, try_project_simple_replication_tx, usage_native_session};
pub use st3_store::store::{usage_period, usage_slot, validate_claim_input, validate_local_observation, work_list};
pub mod agent_messages;
pub mod agent_queue_values;
pub mod assistant_write;
pub mod assistant_write_gate;
pub mod conversation_reads;
pub mod coordination;
pub mod custom;
pub mod delivery;
pub mod directive_notes;
pub mod glasses;
pub mod mailbox_changes;
pub mod mailbox_lease;
pub mod mailbox_wakes;
pub mod message_bodies;
pub mod projection_divergence;
pub mod unread_mail;
pub mod work_summaries_ivm;
#[cfg(test)]
pub mod checkpoint_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod mailbox_lease_tests_shared_seats;
#[cfg(any(test, feature = "test-support"))]
pub mod mailbox_tests;
// Qualification only: no production caller or Source registration.
#[cfg(test)]
pub mod message_point_qualification;
#[cfg(test)]
pub mod message_send_tests;
#[cfg(test)]
pub mod tests;
#[cfg(test)]
pub mod writer_bound_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod writer_bound_tests_shared_missions;
#[cfg(any(test, feature = "test-support"))]
pub mod terminal_gate_evidence;

/// A single-origin runtime-only history cannot have rival runtime authority. Runtime observations
/// append fields, so an omitted field retains its newest earlier value; explicit null replaces it.
/// Other actual kinds and origins use the general fold and causal source selection instead.
pub fn runtime_only_authority(
    connection: &Connection,
    subject: &str,
    through: u64,
    fields: &[&str],
) -> Result<Option<(Value, String)>> {
    let mut sources = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h0ae5e6c1c684 owner=st3_delivery::store TODO=trace-statement-and-value-representation */ &format!(
        "SELECT DISTINCT claims.kind, claims.origin FROM claims INDEXED BY claims_subject_kind_index
         WHERE claims.subject=?1 AND claims.store_index<=?2 AND {ACTUAL_STATE_CLAIM}
           AND {UNREPAIRED_CLAIM} LIMIT 2"
    ))?;
    let mut rows = sources.query(params![subject, through])?;
    let Some(row) = rows.next()? else { return Ok(None); };
    if row.get::<_, String>(0)? != "runtime.observed" {
        return Ok(None);
    }
    let origin = row.get::<_, String>(1)?;
    if rows.next()?.is_some() {
        return Ok(None);
    }
    let body: String = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h504931f32da7 owner=st3_delivery::store TODO=trace-statement-and-value-representation */ &format!(
        "{} LIMIT 1", newest_claims_of_kind_query("claims.body", "runtime.observed"),
    ))?.query_row(/* sqlite-large-value: allow-for-now lv-h805550de30f9 owner=st3_delivery::store TODO=trace-statement-and-value-representation */ params![subject, through], |row| row.get(0))?;
    let latest: Value = serde_json::from_str(&body)?;
    let latest = latest.get("fields").unwrap_or(&latest);
    let mut older = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h7d40261a11f4 owner=st3_delivery::store TODO=trace-statement-and-value-representation */ &format!(
        "SELECT claims.body FROM claims INDEXED BY claims_subject_kind_accepted_index
         JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 AND claims.kind='runtime.observed' AND claims.store_index<=?4
           AND {UNREPAIRED_CLAIM}
           AND json_type(claims.body, CASE WHEN json_type(claims.body, '$.fields') IS NULL
                         THEN ?2 ELSE ?3 END) IS NOT NULL
         ORDER BY {CANONICAL_ORDER_DESC} LIMIT 1"
    ))?;
    let mut actual = serde_json::Map::new();
    for &field in fields {
        let root_path = format!("$.{field}");
        let fields_path = format!("$.fields.{field}");
        if let Some(value) = latest.get(field) {
            actual.insert(field.into(), value.clone());
        } else {
            let body: Option<String> = older.query_row(
                /* sqlite-large-value: allow-for-now lv-h41dabf229cb0 owner=st3_delivery::store TODO=trace-statement-and-value-representation */ params![subject, root_path, fields_path, through],
                |row| row.get(0),
            ).optional()?;
            if let Some(body) = body {
                let value: Value = serde_json::from_str(&body)?;
                if let Some(value) = value.get("fields").unwrap_or(&value).get(field) {
                    actual.insert(field.into(), value.clone());
                }
            }
        }
    }
    Ok(Some((Value::Object(actual), origin)))
}

/// See [`Store::mailbox_watermark`].
#[derive(Clone, Debug, PartialEq)]
pub struct MailboxWatermark {
    index: u64,
    local: i64,
    owner: Option<(String, i64, bool)>,
    desired: Option<String>,
}

fn message_reminder_rank(tags: &[String]) -> Option<(&str, u64)> {
    let reminder = tags.iter().find_map(|tag| tag.strip_prefix("reminder:"))?;
    let version = tags
        .iter()
        .find_map(|tag| tag.strip_prefix("version:"))
        .and_then(|version| version.parse::<u64>().ok())
        .unwrap_or_default();
    Some((reminder, version))
}

fn selected_actionable_messages(
    connection: &Connection,
    messages: Vec<MessageView>,
) -> Result<Vec<MessageView>> {
    let mut selected_reminders = BTreeMap::<String, (u64, MessageView)>::new();
    let mut selected = Vec::new();
    for message in messages {
        let Some((reminder, version)) = message_reminder_rank(&message.tags) else {
            selected.push(message);
            continue;
        };
        let replace = selected_reminders
            .get(reminder)
            .is_none_or(|(current, selected)| {
                version > *current || (version == *current && message.subject > selected.subject)
            });
        if replace {
            selected_reminders.insert(reminder.to_owned(), (version, message));
        }
    }
    selected.extend(selected_reminders.into_values().map(|(_, message)| message));
    sort_messages_canonically(connection, &mut selected)?;
    Ok(selected)
}

pub trait DeliveryStoreCore {
    fn append_assistant_claim_outcome(
            &self,
            input: &ClaimInput,
            authorization: &assistant_write::AssistantWriteAuthorization,
        ) -> Result<(ClaimRecord, bool), St3Error>;
    fn append_claim_fenced_outcome(
            &self,
            input: &ClaimInput,
            fence: Option<&crate::mailbox::Fence>,
        ) -> Result<(ClaimRecord, bool), St3Error>;
    fn unread_mail_count_before(&self, before_unix_ms: u128) -> Result<u64>;
    /// Each seat's unread held mail sent before the cutoff: its count and oldest send time.
    /// A read: it never folds the unread queue, and reads the queued subjects as they stand.
    fn held_mail_before(&self, before_unix_ms: u128) -> Result<Vec<(String, u64, u128)>>;
}

impl DeliveryStoreCore for Store {

    fn append_assistant_claim_outcome(
            &self,
            input: &ClaimInput,
            authorization: &assistant_write::AssistantWriteAuthorization,
        ) -> Result<(ClaimRecord, bool), St3Error> {
        if input.kind != "message.sent" || authorization.scope != "write.assistant.messages"
            || authorization.recipient.as_deref() != input.fields.get("to").and_then(Value::as_str)
            || input.actor.as_deref() != Some("assistant/chatgpt")
            || input.fields.get("from").and_then(Value::as_str) != Some("assistant/chatgpt")
        { return Err(St3Error::new("forbidden", "assistant authorization does not bind this message")); }
        #[cfg(feature = "test-support")]
        let _gate = assistant_write_gate::pause(&authorization.grant_subject)?;
        append_claim_with_commit_context(&self.graph, input, None, None, None, None,
            ClaimCommitContext { assistant: Some(authorization), ..Default::default() })
    }

    fn append_claim_fenced_outcome(
            &self,
            input: &ClaimInput,
            fence: Option<&crate::mailbox::Fence>,
        ) -> Result<(ClaimRecord, bool), St3Error> {
        append_claim_fenced_outcome(&self.graph, input, fence)
    }

    fn unread_mail_count_before(&self, before_unix_ms: u128) -> Result<u64> {
        smallclaims::touched::note_read(|| "kind:message.sent".to_owned());
        if self.readers.has_pinned_reader() {
            return unread_mail::count_in_snapshot(&self.readers.get(), before_unix_ms);
        }
        let mut connection = self.connection.write();
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let count = unread_mail::count_before(&transaction, before_unix_ms)?;
        transaction.commit()?;
        Ok(count)
    }

    fn held_mail_before(&self, before_unix_ms: u128) -> Result<Vec<(String, u64, u128)>> {
        let mut seats = BTreeMap::<String, (u64, u128)>::new();
        for sent in unread_mail::sent_before(&self.readers.get(), before_unix_ms)? {
            if sent
                .tags
                .iter()
                .any(|tag| crate::silent::is_silent_tag(tag))
                && !crate::silent::always_wakes(&sent.from, &sent.tags)
            {
                let seat = seats.entry(sent.to).or_insert((0, sent.sent_unix_ms));
                seat.0 += 1;
                seat.1 = seat.1.min(sent.sent_unix_ms);
            }
        }
        Ok(seats
            .into_iter()
            .map(|(seat, (count, oldest))| (seat, count, oldest))
            .collect())
    }
}

