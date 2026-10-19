//! The part of `store` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared_seats::{BASE_REBUILDS, DEFINITIONS_PARSED, RUN_PART_RUNS, STATUS_AFTER_DESIRED_READ, STEPS_ENRICHED, SUBJECT_REDUCTIONS};
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared::*;
use crate::AttentionSnapshotStore;
use crate::AttentionStore;
use crate::CardFoldStore;
use crate::CheckpointRulesStore;
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
#[cfg(test)]
#[cfg(any(test, feature = "test-support"))]
use smallclaims::replication::ReplicationBatch;
use crate::model::{SeatQueueMoveRequest, SeatQueueMoveView};
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
pub use attention_snapshot::{
    CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, native_prompt_gone_key,
    plan_approval, screen_prompt_delivered_key, screen_prompt_gone_key,
    screen_prompt_undelivered_key,
};
pub use st3_store::store::mission_eligibility::MISSING_AGENT_CONDITION;
pub use st3_store::store::database_size::DatabaseSize;
pub use st3_store::store::person_work::AskImages;
pub use checkpoint_rules::{RULES_VERSION, plan_drops, rules_digest};
pub use st3_store::store::limits::{AccountLimit, LIMITS_CATCH_UP_PAGE};
pub use st3_store::store::limits::ACCOUNT_READING_WINDOW_MS;
pub use st3_store::store::subagents::{SUBAGENT_KINDS, SubagentView};
pub use runtime::SmalltalkRuntime;
#[cfg(any(test, feature = "test-support"))]
pub use smallclaims::sqlite::STATEMENTS_RUN;
use crate::resource::newer_comment;
use st3_kernel::model::LOCAL_OBSERVATION_ID_PREFIX;
pub use st3_kernel::model::local_observation_position;
pub use crate::incremental::{Change, ChangeFeed};
pub use crate::model::canonical_child_string;
pub use st3_store::store::*;
pub use st3_store::store::{ACTUAL_CACHE_LIMIT, AgentStatusEntry, AgentWorkQueue, CachedUsageFold, ChildMissionContext, ClaimAdmission, ClaimCommitContext, ClaimsReduction, CumulativeUsage, DesiredRow, DiscoveredItem, EffectiveStepOwner};
pub use st3_store::store::{INCARNATION_OF_CLAIM, IdempotencyConflict, LOCAL_OBSERVATION_COLUMNS, MAX_STEP_WAIT_MS, MailboxWatermark, MessageCacheEntry, PinnedDefinitions, RANGE_SUBJECTS, RECENTLY_ENDED_MS, RUN_REPORT_KIND, RecordedSeatMove, ReferenceGraph};
pub use st3_store::store::{SeatQueueInputs, StatusEntry, StepLabel, StepTiming, Store, SubjectCache, TERMINAL_OWNED_RUNTIME_SUBJECTS, UNREPAIRED_CLAIM, USAGE_BUCKETS, USAGE_RESPONSE_HORIZON_MS, UsageFold, UsageFolds};
pub use st3_store::store::{UsageSpend, ViewEntry, WORK_NUDGED_KIND, adhoc_work, agent_messages, agent_queue_values, analyze_mission_revision, append_claim_fenced_outcome, append_claim_tx, append_claim_with_commit_context, append_claim_with_fences, append_latest_observation_publication};
pub use st3_store::store::{append_receipt_claim_tx, apply_effective_step_state, apply_effective_step_state_with_owner, arrangements, assistant_write, backup_format, carried_revision_step_paths, check_mailbox_fence, checkpoint, checkpoint_status, claim_append_error, claim_by_id_tx};
pub use st3_store::store::{claim_log_order, classify_replicated_claim_with_registry, compatible_step_paths, conversation_reads, coordination, current_desired_row, current_desired_row_tx, current_harness_at, database_size, declarations, delegation, delivery};
pub use st3_store::store::{desired_from_row, desired_lineage_from_tx, desired_revision, end_watch_tx, enrich_step_definition, enrich_step_summaries_at, evidence, fold_step_clock, fold_step_timing, format_wait_span, generation_id_from_subject, git_change_tables};
pub use st3_store::store::{github_post_agent_tx, github_workflow_failures, glass_heads, harness_fold_at, harness_questions, idle_mail, incarnation_claim_time_query, insert_event, insert_local_observation_tx, intent_leaves_tx, is_failed_terminal, lanes};
pub use st3_store::store::{latest_actual, latest_retention, limits, local_observation_dedupe_key, local_observation_from_row, local_retention, loop_run_views_tx, mailbox_changes, mailbox_lease, mailbox_wakes, message_bodies, message_subscriptions};
pub use st3_store::store::{migrate_local_usage_seen, mission_eligibility, mission_ivm, mission_list, mission_provenance, mission_run_header_tx, mission_run_outcome_tx, mission_run_variables, missions, newest_claims_of_kind_query, normalize_message_party, normalize_seat};
pub use st3_store::store::{observer_actuals, observer_subscription_detail, open_mission_run_headers_query, operation_audit, person_work, pricing_identity, project_claim_isolated_tx, project_mission_run_created, project_mission_run_update, project_replicated_mission_runs, projection_digest, projection_divergence};
pub use st3_store::store::{projectors, publication, publication_facts, rebuild_planning_tx, recently_ended_since, reconcile_work_context, replay_graph_from_nothing_tx, resources, retain_reopened_step_paths, revision_seats, rollouts, roster_admission};
pub use st3_store::store::{run_report_fields, runtime_list, runtime_starts, same_declared_launch, schema_fields_for_body, seat_queue_inputs_tx, seat_queue_joins_query, seat_queue_moves_query, seat_work_at_snapshot_query, select_replicated_desired, select_replicated_mission, selected_work_at_snapshot_query};
pub use st3_store::store::{stale_pull_request_request_tx, step_execution_clock_at, step_labels, step_run_from_row, step_timeout_extension_at, step_timing_lease_at, subagents, subscription_started_run_tx, system_local_retention, trace_principal, unread_mail, usage_native_session};
pub use st3_store::store::{usage_period, usage_slot, validate_claim_input, validate_local_observation, watch_ended_tx, work_at_snapshot_query, work_list};
pub mod accounts;
pub mod agent_view;
pub mod attention;
#[cfg_attr(
    not(any(test, feature = "test-support")),
    expect(
        dead_code,
        reason = "prepared attention operators await certified shared-runtime integration"
    )
)]
pub mod attention_ivm;
pub mod attention_list;
pub mod attention_snapshot;
pub mod backup;
pub mod card_fold;
pub mod checkpoint_rules;
pub mod client_summary;
pub mod custom;
pub mod directive_notes;
pub mod glasses;
pub mod operations;
pub mod owned_sets;
pub mod owner_lists;
pub mod published_list;
pub mod published_views;
pub mod resource_preparation;
pub mod retirements;
pub mod runtime;
pub mod seat_status;
pub mod seats;
pub mod stop_deferral;
pub mod summary_list;
pub mod work_summaries_ivm;
#[cfg(test)]
pub mod actual_state_sorter_tests;
#[cfg(test)]
pub mod arrangements_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod attention_snapshot_tests_shared_seats;
#[cfg(test)]
pub mod card_fold_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod card_fold_tests_shared;
#[cfg(test)]
pub mod checkpoint_agreement_tests;
#[cfg(test)]
pub mod checkpoint_capture_epoch_tests;
#[cfg(test)]
pub mod checkpoint_replication_capture_tests;
#[cfg(test)]
pub mod checkpoint_tests;
#[cfg(test)]
pub mod convergence;
#[cfg(test)]
pub mod fleet_admission_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod mission_ivm_tests_shared_seats;
#[cfg(any(test, feature = "test-support"))]
pub mod seat_status_tests_shared_seats;
#[cfg(test)]
pub mod tests;
#[cfg(any(test, feature = "test-support"))]
pub mod trace_principal_tests_shared_seats;
#[cfg(test)]
pub mod usage_fold_tests;
#[cfg(test)]
pub mod owned_sets_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod terminal_gate_evidence;

/// How many threads one large status projection splits across.
const STATUS_WORKERS: usize = 4;

/// The predicate of `claims_current_view_index`, word for word, so SQLite can use the index.
const CURRENT_VIEW_CLAIM: &str = "(kind='intent.desired'
   OR json_extract(body, '$.fields.status') NOT IN ('stopped', 'absent', 'exited')
   OR json_extract(body, '$.status') NOT IN ('stopped', 'absent', 'exited'))";

/// The subjects with a runtime observation at or before store index `?1`, in subject order. Each
/// step seeks past the subject before it in `claims_runtime_subject_index`, so the query reads
/// about one index entry per subject, not every runtime observation.
pub const RUNTIME_SUBJECTS: &str = "WITH RECURSIVE runtime_subjects(subject) AS (
         SELECT (SELECT subject FROM claims INDEXED BY claims_runtime_subject_index
                 WHERE kind='runtime.observed' AND store_index<=?1 ORDER BY subject LIMIT 1)
         UNION ALL
         SELECT (SELECT subject FROM claims INDEXED BY claims_runtime_subject_index
                 WHERE kind='runtime.observed' AND subject>runtime_subjects.subject
                   AND store_index<=?1
                 ORDER BY subject LIMIT 1)
         FROM runtime_subjects WHERE runtime_subjects.subject IS NOT NULL
     )
     SELECT subject FROM runtime_subjects WHERE subject IS NOT NULL";

/// The least string above every string that begins with `prefix`, when `prefix` is plain ASCII
/// without GLOB wildcards, so `subject GLOB 'prefix*'` holds exactly when
/// `prefix <= subject < bound`.
fn prefix_upper_bound(prefix: &str) -> Option<String> {
    if !prefix.is_ascii() || prefix.contains(['*', '?', '[']) {
        return None;
    }
    let mut bytes = prefix.as_bytes().to_vec();
    let last = bytes.last_mut().filter(|last| **last < 0x7f)?;
    *last += 1;
    String::from_utf8(bytes).ok()
}

/// Whether `subject` names a runtime, whose stopped or undeclared state puts it in history.
fn runtime_subject(subject: &str) -> bool {
    subject.starts_with("agent/")
        || subject.starts_with("exec/")
        || subject.starts_with("pty/")
        || subject.starts_with("gate-operation/")
}

const AGENT_CARD_STATUS_LIMIT: usize = 4096;

fn subject_head_at(connection: &Connection, subject: &str, store_index: u64) -> Result<u64> {
    if subject.starts_with("arrangement/") {
        let index: u64 = connection.query_row("SELECT changed_index FROM arrangements WHERE subject=?1", [subject], |row| row.get(0)).optional()?.unwrap_or(0);
        anyhow::ensure!(index <= store_index, "arrangement snapshot frontier is stale");
        return Ok(index);
    }
    connection
        .prepare_cached(
            "SELECT COALESCE(MAX(store_index), 0) FROM claims WHERE subject=?1 AND store_index<=?2",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-ha1e54170d097 owner=st3_attention::store TODO=trace-statement-and-value-representation */ params![subject, store_index], |row| row.get::<_, u64>(0))
        .map_err(Into::into)
}

/// [`runtime_view_entry`] from a runtime's head and whether it has a current-view claim. Its
/// declaration, the actual states of it and its owners, and its owners' heads are read through
/// the closures, only as the answer needs them.
#[allow(clippy::too_many_arguments)]
fn runtime_view_entry_from(
    connection: &Connection,
    subject: &str,
    store_index: u64,
    owners: bool,
    head: u64,
    candidate: bool,
    desired: &mut dyn FnMut() -> Result<Option<DesiredRow>>,
    actual: &mut dyn FnMut(&str) -> Result<Option<Value>>,
    owner_head: &mut dyn FnMut(&str) -> Result<u64>,
) -> Result<ViewEntry> {
    let entry = |history| ViewEntry {
        head,
        history,
        declared: false,
        owners: Vec::new(),
    };
    if !candidate {
        return Ok(entry(true));
    }
    let status = |actual: Option<Value>| -> Option<String> {
        actual
            .as_ref()
            .and_then(|value| value.get("fields").unwrap_or(value).get("status"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let stopped = |status: Option<&str>| matches!(status, Some("stopped" | "absent" | "exited"));
    let Some(desired) = desired()? else {
        // Undeclared: current only while its status is live.
        let status = status(actual(subject)?);
        return Ok(entry(status.is_none() || stopped(status.as_deref())));
    };
    if !owners {
        return Ok(ViewEntry {
            declared: true,
            ..entry(false)
        });
    }
    let annotation = operational_annotation(
        connection,
        subject,
        Some(&desired.kind),
        true,
        desired.owner_run.as_deref(),
        desired.owner_generation.as_deref(),
        None,
        Some(store_index),
        actual,
    )?;
    // A stop declaration leaves a stopped runtime in history; no other declaration reads status.
    let history = annotation.layer != "current"
        || (desired.kind == "stop" && stopped(status(actual(subject)?).as_deref()));
    let owners = desired
        .owner_run
        .iter()
        .chain(desired.owner_generation.iter())
        .cloned()
        .collect::<Vec<_>>();
    let mut head = head;
    for owner in &owners {
        head = head.max(owner_head(owner)?);
    }
    Ok(ViewEntry {
        head,
        history,
        declared: true,
        owners,
    })
}

/// One subject's status at `at_index`, and the action it asks of its host when it is current and
/// differs from what is declared. With `owner_filter`, a subject another run owns is skipped.
pub fn subject_status_at(
    connection: &Connection,
    subject: &str,
    at_index: Option<u64>,
    owner_filter: Option<&str>,
    claims: ClaimsReduction,
) -> Result<Option<(SubjectStatus, Option<PlannedAction>)>> {
    let _profile = crate::profile::span(match claims {
        ClaimsReduction::Full => "status/reduction-full",
        ClaimsReduction::Summary => "status/reduction-summary",
        ClaimsReduction::Runtime => "status/reduction-runtime",
        ClaimsReduction::AgentCard => "status/reduction-agent-card",
    });
    #[cfg(any(test, feature = "test-support"))]
    SUBJECT_REDUCTIONS.with(|reductions| reductions.set(reductions.get() + 1));
    if subject.starts_with("arrangement/") {
        if owner_filter.is_some() { return Ok(None); }
        let actual = arrangements::arrangement_at(connection, subject, at_index.unwrap_or(i64::MAX as u64))?;
        let revision = actual.as_ref().and_then(|v| v["revision"].as_str()).map(str::to_owned);
        let live = actual.is_some();
        return Ok(Some((SubjectStatus {
            subject: subject.into(), kind: Some("arrangement".into()),
            desired_token: None, desired_revision: None, desired: None,
            actual, actual_claim: revision.clone(), actual_origin: None,
            harness: None, conflicts: vec![], claims: revision.clone().into_iter().collect(),
            claims_summary: ClaimsSummary {
                first: revision.clone(),
                latest: revision.clone(),
                count: usize::from(revision.is_some()),
                session_boundary: None,
            },
            owner_run: None, gap: None, reachability: if live { "reachable" } else { "unknown" }.into(),
            reason: None, under: vec![],
            projection: OperationalAnnotation { layer: if live { "current" } else { "history" }.into(), actionable: false,
                reasons: if live { vec![] } else { vec!["retired-or-uncreated".into()] }, owner_generation: None, runtime_incarnation: None },
        }, None)));
    }
    let desired = desired_row_at(connection, subject, at_index)?;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(after_read) = STATUS_AFTER_DESIRED_READ.with(|slot| slot.borrow_mut().take()) {
        after_read();
    }
    let member = desired
        .as_ref()
        .and_then(|row| row.member.as_deref())
        .and_then(|value| serde_json::from_str::<crate::model::MemberSpec>(value).ok());
    let actual = latest_actual_at(connection, subject, at_index)?;
    let (actual_claim, actual_origin, actual_origin_conflict) = selected_actual_source_at(
        connection,
        subject,
        at_index,
        member.as_ref().map(|member| member.host.as_str()),
    )?;
    let (harness, claims, claims_summary, conflicts) = match claims {
        ClaimsReduction::Full => (
            current_harness_at(connection, subject, at_index)?,
            claim_ids_at(connection, subject, at_index)?,
            ClaimsSummary::default(),
            desired_conflicts_at(
                connection,
                subject,
                desired.as_ref().map(|row| row.claim_id.as_str()),
                at_index,
            )?,
        ),
        ClaimsReduction::Summary => (
            current_harness_at(connection, subject, at_index)?,
            Vec::new(),
            claims_summary_at(connection, subject, at_index)?,
            desired_conflicts_at(
                connection,
                subject,
                desired.as_ref().map(|row| row.claim_id.as_str()),
                at_index,
            )?,
        ),
        ClaimsReduction::Runtime => (
            current_harness_at(connection, subject, at_index)?,
            Vec::new(),
            ClaimsSummary::default(),
            desired_conflicts_at(connection, subject, desired.as_ref().map(|row| row.claim_id.as_str()), at_index)?,
        ),
        ClaimsReduction::AgentCard => {
            let latest = if desired.is_some() {
                None
            } else {
                connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h63138eea82d4 owner=st3_attention::store TODO=trace-statement-and-value-representation */ &canonical_sql(
                    "SELECT id FROM claims INDEXED BY claims_subject_accepted_index
                     WHERE subject=?1 AND store_index<=?2
                     ORDER BY CANONICAL_DESC(claims) LIMIT 1",
                    ))?
                    .query_row(
                        /* sqlite-large-value: allow-for-now lv-hd70a12ee4d30 owner=st3_attention::store TODO=trace-statement-and-value-representation */ params![subject, at_index.unwrap_or(i64::MAX as u64)],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
            };
            (
                None,
                latest.clone().into_iter().collect(),
                ClaimsSummary {
                    latest,
                    ..Default::default()
                },
                Vec::new(),
            )
        }
    };
    if owner_filter.is_some_and(|run| {
        desired.as_ref().and_then(|row| row.owner_run.as_deref()) != Some(run)
    }) {
        return Ok(None);
    }
    let unknown_claim = has_unknown_claim_at(connection, subject, at_index)?;
    let inputs = SubjectStatusInputs {
        desired,
        member,
        actual,
        actual_source: (actual_claim, actual_origin, actual_origin_conflict),
        harness,
        claims,
        claims_summary,
        conflicts,
        unknown_claim,
    };
    assemble_subject_status(
        connection, subject, at_index, inputs,
        &mut |owner| latest_actual_at(connection, owner, at_index),
    ).map(Some)
}

/// What one subject's status reduction reads at a snapshot: read for one subject by
/// [`subject_status_at`], or for many at once by the agent card fold.
pub struct SubjectStatusInputs {
    desired: Option<DesiredRow>,
    member: Option<crate::model::MemberSpec>,
    actual: Option<Value>,
    /// The selected actual claim, its origin, and whether rival origins leave it indeterminate.
    actual_source: (Option<String>, Option<String>, bool),
    harness: Option<crate::model::CurrentHarnessView>,
    claims: Vec<String>,
    claims_summary: ClaimsSummary,
    conflicts: Vec<String>,
    unknown_claim: Option<String>,
}

/// The status and planned action [`SubjectStatusInputs`] reduce to. A snapshot's owning run
/// and generation states are read through `owner_actual`.
fn assemble_subject_status(
    connection: &Connection,
    subject: &str,
    at_index: Option<u64>,
    inputs: SubjectStatusInputs,
    owner_actual: &mut dyn FnMut(&str) -> Result<Option<Value>>,
) -> Result<(SubjectStatus, Option<PlannedAction>)> {
    let SubjectStatusInputs {
        desired,
        member,
        actual,
        actual_source: (actual_claim, actual_origin, actual_origin_conflict),
        harness,
        claims,
        claims_summary,
        conflicts,
        unknown_claim,
    } = inputs;
    let kind = desired.as_ref().map(|row| row.kind.clone());
    let owner_run = desired.as_ref().and_then(|row| row.owner_run.clone());
    let owner_generation = desired
        .as_ref()
        .and_then(|row| row.owner_generation.clone());
    let status = actual
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str);
    let reachability = if unknown_claim.is_some() || actual_origin_conflict {
        "indeterminate".to_owned()
    } else {
        actual
            .as_ref()
            .and_then(|value| value.get("reachability"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if actual.is_some() {
                    "reachable"
                } else if desired.is_some() {
                    "indeterminate"
                } else {
                    // Nothing declares or observes this subject, so nothing says it
                    // can be reached.
                    "unknown"
                }
            })
            .to_owned()
    };
    let gap = match (kind.as_deref(), member.as_ref(), status) {
        (Some("stop"), _, Some("stopped" | "absent" | "exited")) => None,
        (Some("stop"), _, _) => Some("the desired state is stopped".to_owned()),
        (_, Some(_), Some("running" | "ready" | "working" | "idle")) => None,
        (_, Some(member), Some("exited")) if member.restart == crate::model::RestartType::Never => {
            None
        }
        (_, Some(_), Some(value)) => Some(format!("the member is {value}")),
        (_, Some(_), None) => Some("the desired member has no actual state".to_owned()),
        _ => None,
    };
    let projection = operational_annotation(
        connection,
        subject,
        kind.as_deref(),
        desired.is_some(),
        owner_run.as_deref(),
        owner_generation.as_deref(),
        actual.as_ref(),
        at_index,
        owner_actual,
    )?;
    let action = (projection.layer == "current")
        .then_some(gap.as_ref())
        .flatten()
        .map(|reason| PlannedAction {
            subject: subject.to_owned(),
            action: if matches!(kind.as_deref(), Some("stop")) {
                "stop"
            } else {
                "reconcile"
            }
            .into(),
            reason: reason.clone(),
        });
    let reason = actual_origin_conflict
        .then(|| "concurrent runtime observations have indeterminate authority".to_owned())
        .or_else(|| {
            unknown_claim
                .map(|kind| format!("claim kind `{kind}` is not registered"))
                .or_else(|| {
                    actual
                        .as_ref()
                        .and_then(|value| value.get("reason"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
        });
    let desired_value = desired
        .as_ref()
        .map(|row| serde_json::from_str(&row.body))
        .transpose()?;
    let under = desired_value
        .as_ref()
        .map(crate::graph::agent_under)
        .unwrap_or_default();
    Ok((
        SubjectStatus {
            subject: subject.to_owned(),
            kind,
            desired_token: desired.as_ref().map(|row| row.claim_id.clone()),
            desired_revision: desired.as_ref().map(|row| row.revision.clone()),
            desired: desired_value,
            actual,
            actual_claim,
            actual_origin,
            harness,
            conflicts,
            claims,
            claims_summary,
            owner_run,
            gap,
            reachability,
            reason,
            under,
            projection,
        },
        action,
    ))
}

/// The runs that wait on a person, and the open human gates among them.
pub type HumanAttention = (BTreeSet<String>, Vec<st3_ui_model::missions::Gate>);

/// A subject's actual state from already parsed claims in canonical order. It takes
/// the bodies, so each field moves into the folded state instead of being copied.
fn fold_latest_values(rows: impl IntoIterator<Item = (String, Value)>) -> Result<Option<Value>> {
    fold_latest_parsed_values(rows.into_iter().map(Ok))
}

/// The ids sharing one accepted time at the cut, in canonical order. Canonical order inside a
/// single accepted time is decided by the whole claim key, so exactly this group — never the
/// subject's entire history — needs key-by-key resolution.
fn canonical_tie_group_at(
    connection: &Connection,
    subject: &str,
    at_index: u64,
    accepted_at_unix_ms: &str,
) -> Result<Vec<String>> {
    let mut statement = connection.prepare_cached(
        "SELECT id FROM claims INDEXED BY claims_subject_accepted_index
         WHERE subject=?1 AND store_index<=?2 AND accepted_at_unix_ms=?3",
    )?;
    let ids = statement
        .query_map(params![subject, at_index, accepted_at_unix_ms], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut keyed = ids
        .into_iter()
        .map(|id| Ok((canonical::claim_key(connection, &id)?, id)))
        .collect::<Result<Vec<_>>>()?;
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(keyed.into_iter().map(|(_, id)| id).collect())
}

/// The `window`-th oldest canonical claim id, or `None` while the subject has `window` claims
/// or fewer at the cut. Streams the accepted-time index up to the window and resolves
/// canonical keys only for the accepted-time group the window boundary falls inside: every
/// earlier group canonically precedes that group whole, so only the offset within it matters.
fn window_boundary_at(
    connection: &Connection,
    subject: &str,
    at_index: u64,
    window: usize,
) -> Result<Option<String>> {
    let mut statement = connection.prepare_cached(
        "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_subject_accepted_index
         WHERE subject=?1 AND store_index<=?2
         ORDER BY length(accepted_at_unix_ms), accepted_at_unix_ms LIMIT ?3",
    )?;
    let times = statement
        .query_map(params![subject, at_index, window], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if times.len() < window {
        return Ok(None);
    }
    let boundary_time = times.last().expect("a full window has a last time");
    let within_window = times
        .iter()
        .filter(|time| time.as_str() == boundary_time)
        .count();
    let group = canonical_tie_group_at(connection, subject, at_index, boundary_time)?;
    Ok(group.get(within_window - 1).cloned())
}

/// The bounded claims summary [`ClaimsReduction::Summary`] computes instead of the full
/// canonical id vector: the canonically-latest id, the claim count at the cut, and the
/// session-dating window's last canonical id once the count passes it. Each answer reads the
/// subject's accepted-time index once, and canonical tie keys are resolved only for the top
/// tie group and the group the window boundary falls inside, so the cost no longer follows
/// the subject's history beyond the count and window index scans. The statements share one
/// read cut, exactly like `claim_ids_at`.
fn claims_summary_at(
    connection: &Connection,
    subject: &str,
    at_index: Option<u64>,
) -> Result<ClaimsSummary> {
    let snapshot = if connection.is_autocommit() {
        Some(connection.unchecked_transaction()?)
    } else {
        None
    };
    let at_index = at_index.unwrap_or(i64::MAX as u64);
    let count = usize::try_from(
        connection
            .prepare_cached(
                "SELECT COUNT(*) FROM claims INDEXED BY claims_subject_accepted_index
                 WHERE subject=?1 AND store_index<=?2",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-h00c95c1c9fb0 owner=st3_attention::store TODO=trace-statement-and-value-representation */ params![subject, at_index], |row| row.get::<_, i64>(0))?,
    )?;
    // The canonically-first claim: the session list's fallback date for a subject without any
    // other evidence of when it started.
    let first = if count == 0 {
        None
    } else {
        let oldest = connection
            .prepare_cached(
                "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_subject_accepted_index
                 WHERE subject=?1 AND store_index<=?2
                 ORDER BY length(accepted_at_unix_ms), accepted_at_unix_ms LIMIT 1",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-h1abe53ab66ee owner=st3_attention::store TODO=trace-statement-and-value-representation */ params![subject, at_index], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        oldest
            .map(|time| {
                canonical_tie_group_at(connection, subject, at_index, &time)
                    .map(|group| group.into_iter().next())
            })
            .transpose()?
            .flatten()
    };
    let latest = if count == 0 {
        None
    } else {
        let top = connection
            .prepare_cached(
                "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_subject_accepted_index
                 WHERE subject=?1 AND store_index<=?2
                 ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-h683c1077e378 owner=st3_attention::store TODO=trace-statement-and-value-representation */ params![subject, at_index], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        top.map(|time| {
            canonical_tie_group_at(connection, subject, at_index, &time)
                .map(|mut group| group.pop())
        })
        .transpose()?
        .flatten()
    };
    let session_boundary = if count > SESSION_CLAIMS {
        window_boundary_at(connection, subject, at_index, SESSION_CLAIMS)?
    } else {
        None
    };
    if let Some(snapshot) = snapshot {
        snapshot.commit()?;
    }
    Ok(ClaimsSummary {
        first,
        latest,
        count,
        session_boundary,
    })
}

fn desired_conflicts_at(
    connection: &Connection,
    subject: &str,
    winner: Option<&str>,
    at_index: Option<u64>,
) -> Result<Vec<String>> {
    if let Some(conflicts) = owned_sets::conflicts_at(connection, subject, at_index)? {
        return Ok(conflicts);
    }
    let at_index = at_index.unwrap_or(i64::MAX as u64);
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hfc732a103ab9 owner=st3_attention::store TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT id, predecessors FROM claims
         WHERE subject=?1 AND kind='intent.desired' AND store_index<=?2
           AND NOT EXISTS (
               SELECT 1 FROM replica_records
               WHERE replica_records.claim_id=claims.id
                 AND replica_records.state='repaired'
           )
         ORDER BY CANONICAL_ASC(claims)",
    ))?;
    let rows = statement
        .query_map(params![subject, at_index], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let referenced = rows
        .iter()
        .flat_map(|(_, predecessors)| {
            serde_json::from_str::<Vec<String>>(predecessors).unwrap_or_default()
        })
        .collect::<BTreeSet<_>>();
    Ok(rows
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| !referenced.contains(id) && Some(id.as_str()) != winner)
        .collect())
}

fn attention_item_from_failure(request: AttentionRequestView) -> AttentionItemView {
    AttentionItemView {
        episode: request.request.clone(),
        priority: if request.severity == "warning" {
            "normal"
        } else {
            "high"
        }
        .into(),
        kind: "fault".into(),
        review_mode: None,
        subject: request.subject.clone(),
        person: request.reviewer,
        requester_id: Some(request.actor),
        launch_id: None,
        variant_id: None,
        message_id: None, prompt_call: None, answers: Vec::new(), conversation: None,
        title: request.title,
        detail: request.reason,
        mission: None,
        mission_run: None,
        step: None,
        targets: request.targets,
        requested_at_unix_ms: request.requested_at_unix_ms,
        actions: vec![attention_action(
            "inspect source",
            &["st", "subject", &request.subject],
        )],
        request: None,
    }
}

/// A subject's operational annotation. At a snapshot, its owning run's and generation's states
/// are read through `owner_actual`.
#[allow(clippy::too_many_arguments)]
fn operational_annotation(
    connection: &Connection,
    subject: &str,
    desired_kind: Option<&str>,
    has_desired: bool,
    owner_run: Option<&str>,
    owner_generation: Option<&str>,
    actual: Option<&Value>,
    at_index: Option<u64>,
    owner_actual: &mut dyn FnMut(&str) -> Result<Option<Value>>,
) -> Result<OperationalAnnotation> {
    let fields = actual.map(|value| value.get("fields").unwrap_or(value));
    let status = fields
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str);
    let incarnation = fields
        .and_then(|value| value.get("incarnation_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let runtime_subject = runtime_subject(subject);
    let stopped = matches!(status, Some("stopped" | "absent" | "exited"));
    let mut historical = Vec::new();
    if runtime_subject && stopped && (!has_desired || desired_kind == Some("stop")) {
        historical.push("stopped".to_owned());
    }
    // Nothing declares this runtime and no runtime status was ever observed for it, such as an
    // agent known only from its own harness observations. It is a leftover, not a live runtime.
    if runtime_subject && !has_desired && status.is_none() {
        historical.push("undeclared".to_owned());
    }

    if let Some(owner_run) = owner_run {
        let (run_status, current_generation, mode) = if at_index.is_some() {
            let owner = owner_actual(owner_run)?;
            let fields = owner
                .as_ref()
                .map(|value| value.get("fields").unwrap_or(value));
            (
                fields
                    .and_then(|fields| fields.get("status"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                None,
                fields
                    .and_then(|fields| fields.get("mode"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
        } else {
            let run_id = owner_run.strip_prefix("mission-run/").unwrap_or(owner_run);
            connection
                .query_row(
                    "SELECT status, current_generation_id, mode FROM mission_runs WHERE id=?1",
                    [run_id],
                    |row| {
                        Ok((
                            Some(row.get::<_, String>(0)?),
                            Some(format!("run-generation/{}", row.get::<_, String>(1)?)),
                            Some(row.get::<_, String>(2)?),
                        ))
                    },
                )
                .optional()?
                .unwrap_or((None, None, None))
        };
        if run_status.as_deref().is_some_and(is_terminal_run_state) {
            historical.push("terminal-owner".to_owned());
            if mode.as_deref() == Some("eval") {
                historical.push("eval".to_owned());
            }
        }
        if let Some(owner_generation) = owner_generation {
            let generation_status = if at_index.is_some() {
                let generation = owner_actual(owner_generation)?;
                generation
                    .as_ref()
                    .map(|value| value.get("fields").unwrap_or(value))
                    .and_then(|fields| fields.get("status"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            } else {
                let generation_id = generation_id_from_subject(owner_generation);
                connection
                    .query_row(
                        "SELECT status FROM run_generations WHERE id=?1",
                        [generation_id],
                        |row| row.get(0),
                    )
                    .optional()?
            };
            if current_generation
                .as_deref()
                .is_some_and(|current| current != owner_generation)
                || generation_status.as_deref() == Some("superseded")
            {
                historical.push("superseded".to_owned());
            } else if generation_status
                .as_deref()
                .is_some_and(is_terminal_generation_state)
            {
                historical.push("terminal-generation".to_owned());
            }
        }
    }
    historical.sort();
    historical.dedup();
    let layer = if historical.is_empty() {
        "current"
    } else {
        "history"
    };
    let healthy_runtime =
        !runtime_subject || matches!(status, Some("running" | "ready" | "working" | "idle"));
    let mut reasons = historical;
    if layer == "current" && runtime_subject && has_desired && !healthy_runtime {
        reasons.push("unhealthy".into());
    }
    Ok(OperationalAnnotation {
        layer: layer.into(),
        actionable: layer == "current" && healthy_runtime,
        reasons,
        owner_generation: owner_generation.map(str::to_owned),
        runtime_incarnation: incarnation,
    })
}

/// The condition of a broken exec gate's operational failure. Its item goes to the publisher of
/// the run's mission revision: a person reads it in their attention, an agent as a fault.
pub const GATE_BROKEN_CONDITION: &str = "gate-broken";

/// The episode of one attempt's timeout fault. An extension starts a new episode, so a step that
/// runs out of its extended budget faults again.
pub fn step_timeout_episode(subject: &str, attempt: u32, extension_ms: u64) -> String {
    let key = format!("step-timeout:{subject}:{attempt}:{extension_ms}");
    let digest = hex::encode(Sha256::digest(key.as_bytes()));
    format!("attention/{}", &digest[..32])
}

pub trait AttentionStoreCore {
    /// The reason for an open fault on `subject` in `scope`, if the latest record is a fault.
    fn reconcile_fault(&self, subject: &str, scope: &str) -> Result<Option<String>>;
    fn status(&self, selected: Option<&str>) -> Result<StatusResponse>;
    fn status_at(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
        ) -> Result<StatusResponse>;
    fn status_history(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
        ) -> Result<StatusResponse>;
    fn status_for_subject_prefix_uncached(
            &self,
            prefix: &str,
            at_index: Option<u64>,
            include_history: bool,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse>;
    /// Reduce only subjects that have emitted one claim kind at the selected snapshot.
    fn status_for_claim_kind_at(
            &self,
            kind: &str,
            at_index: Option<u64>,
            include_history: bool,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse>;
    /// Of `subjects`, those a current view can show at `store_index`. A runtime that nothing
    /// declares is current only while its folded status is live; the rest is history, and its
    /// reduction reads every claim it has. Each runtime's answer is kept until a claim about it
    /// arrives, so a view costs what it shows and what changed, not every runtime the store has
    /// ever held.
    /// Apply the claims that arrived through `store_index` to the subject cache, and say whether
    /// answers read at `store_index` may be kept: only those at the newest applied snapshot are
    /// known to hold afterwards.
    fn advance_subject_cache(&self, connection: &Connection, store_index: u64) -> Result<bool>;
    /// `subject`'s status at `store_index`, reduced again only when a claim it depends on
    /// arrived since it was last reduced. `newest` says whether to keep a new reduction.
    fn cached_subject_status(
            &self,
            connection: &Connection,
            subject: &str,
            store_index: u64,
            newest: bool,
            claims: ClaimsReduction,
        ) -> Result<(SubjectStatus, Option<PlannedAction>)>;
    /// `subject`'s kept status reduction, if one holds at `store_index`.
    fn kept_subject_status(
            &self, subject: &str, store_index: u64, claims: ClaimsReduction,
        ) -> Option<(SubjectStatus, Option<PlannedAction>)>;
    /// Keep a reduction read at the newest applied snapshot, `store_index`.
    fn keep_subject_status(
            &self, subject: &str, store_index: u64, claims: ClaimsReduction,
            status: &SubjectStatus, action: &Option<PlannedAction>,
        );
    /// Of `subjects`, those a current view can show at `store_index`. A runtime that nothing
    /// declares is current only while its folded status is live, and one whose owning run or
    /// generation ended is history; the rest of history's reduction reads every claim it has.
    /// Each runtime's answer is kept until a claim it depends on arrives, so a view costs what
    /// it shows and what changed, not every runtime the store has ever held. `owners` says
    /// whether the reduction reads owners at the snapshot, as the answers here do.
    fn current_view_candidates(
            &self,
            connection: &Connection,
            subjects: BTreeSet<String>,
            store_index: u64,
            owners: bool,
        ) -> Result<BTreeSet<String>>;
    /// [`Self::current_view_candidates`], reading the runtimes it has no answer for a chunk at
    /// a time into `reads`.
    fn current_view_candidates_with(
            &self,
            connection: &Connection,
            subjects: BTreeSet<String>,
            store_index: u64,
            owners: bool,
            reads: &mut card_fold::CardReads,
        ) -> Result<BTreeSet<String>>;
    fn status_for_subject_names_at(
            &self,
            subjects: BTreeSet<String>,
            store_index: u64,
            include_history: bool,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse>;
    fn status_at_view(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
            include_history: bool,
        ) -> Result<StatusResponse>;
    fn status_at_view_for_names(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
            include_history: bool,
            selected_names: Option<BTreeSet<String>>,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse>;
    fn status_at_view_for_names_in_snapshot(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
            include_history: bool,
            selected_names: Option<BTreeSet<String>>,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse>;
    fn desired_subjects(&self) -> Result<Vec<DesiredSubject>>;
    /// The current faults raised for `person` under the reviewer each one names, owners aside.
    #[cfg(any(test, feature = "test-support"))]
    fn fault_items(&self, person: Option<&str>) -> Result<Vec<AttentionItemView>>;
    /// Every current fault, each with the agent that owns it: the agent assigned to the failed
    /// step, else the run's requester when that is an agent, else the fleet's fault agent. No
    /// fault is a person's: the owning agent retries, revises or cancels, and asks a person
    /// only for what only a person can give.
    fn fault_snapshot(&self, as_of: u128) -> Result<Vec<FaultView>>;
    fn signature_attention_items(&self) -> Result<Vec<AttentionItemView>>;
    fn selected_desired_token(&self, subject: &str) -> Result<Option<String>>;
    fn selected_desired_revision(&self, subject: &str) -> Result<Option<String>>;
}

impl AttentionStoreCore for Store {

    fn reconcile_fault(&self, subject: &str, scope: &str) -> Result<Option<String>> {
        let connection = self.readers.get();
        let body: Option<String> = connection
            .query_row(
                /* sqlite-large-value: allow-for-now lv-h0a6d0f5ada00 owner=st3_attention::store TODO=trace-statement-and-value-representation */ &canonical_sql(
                    "SELECT body FROM claims WHERE subject=?1 AND kind='reconcile.fault'
                   AND json_extract(body, '$.fields.scope')=?2
                 ORDER BY CANONICAL_DESC(claims) LIMIT 1",
                ),
                params![subject, scope],
                |row| row.get(0),
            )
            .optional()?;
        let Some(body) = body else {
            return Ok(None);
        };
        let body: Value = serde_json::from_str(&body)?;
        let fields = body.get("fields").unwrap_or(&body);
        Ok(
            (fields.get("status").and_then(Value::as_str) == Some("faulted")).then(|| {
                fields
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            }),
        )
    }

    fn status(&self, selected: Option<&str>) -> Result<StatusResponse> {
        self.status_at_view(selected, None, None, false)
    }

    fn status_at(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
        ) -> Result<StatusResponse> {
        self.status_at_view(selected, selected_owner_run, at_index, false)
    }

    fn status_history(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
        ) -> Result<StatusResponse> {
        self.status_at_view(selected, selected_owner_run, at_index, true)
    }

    fn status_for_subject_prefix_uncached(
            &self,
            prefix: &str,
            at_index: Option<u64>,
            include_history: bool,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse> {
        let connection = self.readers.get();
        let current = current_index(&connection)?;
        let store_index = selected_index(current, at_index).map_err(anyhow::Error::new)?;
        let subjects = if prefix.starts_with("arrangement/") {
            connection.prepare_cached("SELECT subject FROM arrangements WHERE subject>=?1 AND subject<?2 ORDER BY subject")?
                .query_map(params![prefix, prefix_upper_bound(prefix).context("arrangement prefix bound")?], |row| row.get::<_,String>(0))?
                .collect::<Result<BTreeSet<_>,_>>()?
        } else { match prefix_upper_bound(prefix) {
            // One seek per subject instead of every claim of every subject in the range.
            Some(bound) => connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-hf1270d97718b owner=st3_attention::store TODO=trace-statement-and-value-representation */ RANGE_SUBJECTS)?
                .query_map(params![store_index, prefix, bound], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<BTreeSet<_>, _>>()?,
            None => connection
                .prepare(
                    "SELECT DISTINCT subject FROM claims
                     WHERE store_index<=?1 AND subject GLOB ?2 ORDER BY subject",
                )?
                .query_map(params![store_index, format!("{prefix}*")], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<BTreeSet<_>, _>>()?,
        }};
        drop(connection);
        self.status_for_subject_names_at(subjects, store_index, include_history, claims)
    }

    fn status_for_claim_kind_at(
            &self,
            kind: &str,
            at_index: Option<u64>,
            include_history: bool,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse> {
        let connection = self.readers.get();
        let current = current_index(&connection)?;
        let store_index = selected_index(current, at_index).map_err(anyhow::Error::new)?;
        let subjects = if kind == "arrangement.edited" {
            connection.prepare_cached("SELECT subject FROM arrangements ORDER BY subject")?
                .query_map([], |row| row.get::<_,String>(0))?.collect::<Result<BTreeSet<_>,_>>()?
        } else if kind == "runtime.observed" {
            // One seek per runtime instead of every runtime observation.
            connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-h15406d07736f owner=st3_attention::store TODO=trace-statement-and-value-representation */ RUNTIME_SUBJECTS)?
                .query_map([store_index], |row| row.get::<_, String>(0))?
                .collect::<Result<BTreeSet<_>, _>>()?
        } else {
            connection
                .prepare(
                    "SELECT DISTINCT subject FROM claims
                     WHERE kind=?1 AND store_index<=?2 ORDER BY subject",
                )?
                .query_map(params![kind, store_index], |row| row.get::<_, String>(0))?
                .collect::<Result<BTreeSet<_>, _>>()?
        };
        drop(connection);
        self.status_for_subject_names_at(subjects, store_index, include_history, claims)
    }

    fn advance_subject_cache(&self, connection: &Connection, store_index: u64) -> Result<bool> {
        let mut cache = self
            .smalltalk
            .subject_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if store_index > cache.through {
            let conflicts = connection
                .prepare_cached("SELECT COUNT(*) FROM operations WHERE state='conflict'")?
                .query_row(/* sqlite-large-value: allow-for-now lv-ha0781fcb2ef0 owner=st3_attention::store TODO=trace-statement-and-value-representation */ [], |row| row.get::<_, u64>(0))?;
            if conflicts != cache.conflicts {
                cache.views.clear();
                cache.statuses.clear();
                cache.summary_statuses.clear();
                cache.runtime_statuses.clear();
                cache.card_statuses.clear();
                cache.conflicts = conflicts;
            } else if !cache.views.is_empty() || !cache.statuses.is_empty()
                || !cache.summary_statuses.is_empty() || !cache.runtime_statuses.is_empty()
                || !cache.card_statuses.is_empty() {
                // The subjects and actors of the claims that arrived.
                let mut statement = connection.prepare_cached(
                    "SELECT subject, actor FROM claims WHERE store_index>?1 AND store_index<=?2",
                )?;
                let mut changed = HashSet::new();
                for row in statement.query_map(params![cache.through, store_index], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })? {
                    let (subject, actor) = row?;
                    changed.insert(subject);
                    changed.extend(actor);
                }
                drop(statement);
                let stale = |subject: &String, owners: &[String]| {
                    changed.contains(subject) || owners.iter().any(|owner| changed.contains(owner))
                };
                cache
                    .views
                    .retain(|subject, entry| !stale(subject, &entry.owners));
                cache
                    .statuses
                    .retain(|subject, entry| !stale(subject, &entry.owners));
                cache.summary_statuses.retain(|subject, entry| !stale(subject, &entry.owners));
                cache.runtime_statuses.retain(|subject, entry| !stale(subject, &entry.owners));
                cache.card_statuses.retain(|subject, entry| !stale(subject, &entry.owners));
            }
            cache.through = store_index;
        }
        Ok(store_index == cache.through)
    }

    fn cached_subject_status(
            &self,
            connection: &Connection,
            subject: &str,
            store_index: u64,
            newest: bool,
            claims: ClaimsReduction,
        ) -> Result<(SubjectStatus, Option<PlannedAction>)> {
        if let Some(kept) = self.kept_subject_status(subject, store_index, claims) {
            return Ok(kept);
        }
        let (status, action) =
            subject_status_at(connection, subject, Some(store_index), None, claims)?
                .expect("a reduction without an owner filter always has a status");
        if newest {
            self.keep_subject_status(subject, store_index, claims, &status, &action);
        }
        Ok((status, action))
    }

    fn kept_subject_status(
            &self, subject: &str, store_index: u64, claims: ClaimsReduction,
        ) -> Option<(SubjectStatus, Option<PlannedAction>)> {
        let cache = self
            .smalltalk
            .subject_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // A summary and a full reduction of one subject are kept side by side; a public
        // response must never be served a summary.
        let entries = match claims {
            ClaimsReduction::Full => &cache.statuses,
            ClaimsReduction::Summary => &cache.summary_statuses,
            ClaimsReduction::Runtime => &cache.runtime_statuses,
            ClaimsReduction::AgentCard => &cache.card_statuses,
        };
        entries.get(subject)
            .filter(|entry| {
                entry.claims == claims
                    && entry.read_at <= store_index
                    && store_index <= cache.through
            })
            .map(|entry| (entry.status.clone(), entry.action.clone()))
    }

    fn keep_subject_status(
            &self, subject: &str, store_index: u64, claims: ClaimsReduction,
            status: &SubjectStatus, action: &Option<PlannedAction>,
        ) {
        let mut cache = self
            .smalltalk
            .subject_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Keep it only if no later snapshot was applied meanwhile.
        if cache.through != store_index {
            return;
        }
        let owners = status
            .owner_run
            .iter()
            .chain(status.projection.owner_generation.iter())
            .cloned()
            .collect();
        let entries = match claims {
            ClaimsReduction::Full => &mut cache.statuses,
            ClaimsReduction::Summary => &mut cache.summary_statuses,
            ClaimsReduction::Runtime => &mut cache.runtime_statuses,
            ClaimsReduction::AgentCard => &mut cache.card_statuses,
        };
        // At capacity, a cold summary remains correct and uncached. Invalidation
        // frees slots; no growing history map or full-status cache warming is needed.
        if matches!(claims, ClaimsReduction::AgentCard)
            && entries.len() >= AGENT_CARD_STATUS_LIMIT && !entries.contains_key(subject) {
            return;
        }
        entries.insert(
            subject.to_owned(),
            StatusEntry {
                read_at: store_index,
                owners,
                claims,
                status: status.clone(),
                action: action.clone(),
            },
        );
    }

    fn current_view_candidates(
            &self,
            connection: &Connection,
            subjects: BTreeSet<String>,
            store_index: u64,
            owners: bool,
        ) -> Result<BTreeSet<String>> {
        let mut reads = card_fold::CardReads::new(store_index);
        self.current_view_candidates_with(connection, subjects, store_index, owners, &mut reads)
    }

    fn current_view_candidates_with(
            &self,
            connection: &Connection,
            subjects: BTreeSet<String>,
            store_index: u64,
            owners: bool,
            reads: &mut card_fold::CardReads,
        ) -> Result<BTreeSet<String>> {
        let newest = self.advance_subject_cache(connection, store_index)?;
        let mut candidates = BTreeSet::new();
        let mut unknown = Vec::new();
        {
            let cache = self
                .smalltalk
                .subject_cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for subject in subjects {
                if !runtime_subject(&subject) {
                    candidates.insert(subject);
                    continue;
                }
                // Without the owner checks, only an answer that used no declaration applies.
                let known = cache
                    .views
                    .get(&subject)
                    .filter(|entry| entry.head <= store_index && (owners || !entry.declared))
                    .map(|entry| entry.history);
                match known {
                    Some(false) => {
                        candidates.insert(subject);
                    }
                    Some(true) => {}
                    None => unknown.push(subject),
                }
            }
        }
        let mut entries =
            self.runtime_view_entries(connection, &unknown, store_index, owners, reads)?;
        for subject in unknown {
            let entry = entries
                .remove(&subject)
                .expect("every runtime read has a view entry");
            let history = entry.history;
            if newest && owners {
                let mut cache = self
                    .smalltalk
                    .subject_cache
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if cache.through == store_index {
                    cache.views.insert(subject.clone(), entry);
                }
            }
            if !history {
                candidates.insert(subject);
            }
        }
        Ok(candidates)
    }

    fn status_for_subject_names_at(
            &self,
            subjects: BTreeSet<String>,
            store_index: u64,
            include_history: bool,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse> {
        self.readers.request_read(|| {
            let subjects = if include_history {
                subjects
            } else {
                self.current_view_candidates(&self.readers.get(), subjects, store_index, true)?
            };
            if subjects.len() <= 64 {
                return self.status_at_view_for_names(
                    None, None, Some(store_index), include_history, Some(subjects), claims,
                );
            }
            // The caller keeps its loan and reduces the LAST slice on it, so a full
            // four-way split uses three extra permits and no extra connection for the
            // parent. Other slices must never wait for admission: when a permit is
            // unavailable they also reduce inline on that same loan. Slices stay in
            // subject order regardless of which ones execute in parallel.
            enum Part<'scope> {
                Worker(std::thread::ScopedJoinHandle<'scope, Result<StatusResponse>>),
                Inline(Result<StatusResponse>),
            }
            let subjects = subjects.into_iter().collect::<Vec<_>>();
            let chunk_size = subjects.len().div_ceil(STATUS_WORKERS);
            let profile = crate::profile::current();
            let read_budget = smallclaims::read_budget::current();
            let parts = std::thread::scope(|scope| {
                subjects
                    .chunks(chunk_size)
                    .enumerate()
                    .map(|(position, chunk)| {
                        let names = chunk.iter().cloned().collect::<BTreeSet<_>>();
                        let read = move || self.status_at_view_for_names(
                            None, None, Some(store_index), include_history, Some(names), claims,
                        );
                        let last = position + 1 == subjects.len().div_ceil(chunk_size);
                        if !last && let Some(permit) = self.readers.try_admit_read() {
                            let profile = profile.clone();
                            let read_budget = read_budget.clone();
                            Part::Worker(scope.spawn(move || {
                                let _entered = crate::profile::enter(profile.as_ref());
                                smallclaims::read_budget::with(read_budget, || {
                                    self.readers.request_read_with_permit(permit, read)?
                                })
                            }))
                        } else {
                            Part::Inline(read())
                        }
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|part| match part {
                        Part::Worker(worker) => worker.join().expect("status projection worker panicked"),
                        Part::Inline(result) => result,
                    })
                    .collect::<Vec<_>>()
            });
            let mut merged = StatusResponse {
                store_index,
                subjects: Vec::new(),
                pending_actions: Vec::new(),
            };
            for part in parts {
                let part = part?;
                merged.subjects.extend(part.subjects);
                merged.pending_actions.extend(part.pending_actions);
            }
            Ok(merged)
        })?
    }

    fn status_at_view(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
            include_history: bool,
        ) -> Result<StatusResponse> {
        self.status_at_view_for_names(
            selected,
            selected_owner_run,
            at_index,
            include_history,
            None,
            // The response a public status endpoint serializes carries the full claims vector.
            ClaimsReduction::Full,
        )
    }

    fn status_at_view_for_names(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
            include_history: bool,
            selected_names: Option<BTreeSet<String>>,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse> {
        // A request reader loan alone can see a commit between statements. Keep the
        // selected declaration, runtime observation and conflict leaves in one snapshot.
        self.read_snapshot(|_| {
            self.status_at_view_for_names_in_snapshot(
                selected,
                selected_owner_run,
                at_index,
                include_history,
                selected_names,
                claims,
            )
        })
    }

    fn status_at_view_for_names_in_snapshot(
            &self,
            selected: Option<&str>,
            selected_owner_run: Option<&str>,
            at_index: Option<u64>,
            include_history: bool,
            selected_names: Option<BTreeSet<String>>,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse> {
        let connection = self.readers.get();
        let current = current_index(&connection)?;
        let store_index = selected_index(current, at_index).map_err(anyhow::Error::new)?;
        let bounded_names = selected_names.is_some();
        let mut subject_names = selected_names.unwrap_or_default();
        if let Some(selected) = selected {
            subject_names.insert(selected.to_owned());
        } else if !bounded_names {
            let mut statement = connection.prepare(
                "SELECT DISTINCT subject FROM claims WHERE store_index<=?1 ORDER BY subject",
            )?;
            let rows = statement.query_map([store_index], |row| row.get::<_, String>(0))?;
            subject_names.extend(rows.collect::<Result<Vec<_>, _>>()?);
            drop(statement);
            if !include_history {
                // Without a snapshot the reduction reads owners from the run tables, so only
                // what a runtime's own claims decide leaves it out.
                subject_names = self.current_view_candidates(
                    &connection,
                    subject_names,
                    store_index,
                    at_index.is_some(),
                )?;
            }
        }
        // A snapshot reduction is kept per subject until a claim it depends on arrives.
        let newest = match at_index {
            Some(_) if selected_owner_run.is_none() => {
                Some(self.advance_subject_cache(&connection, store_index)?)
            }
            _ => None,
        };
        let mut subjects = Vec::new();
        let mut pending_actions = Vec::new();
        for subject in subject_names {
            if subject.starts_with("glass/") || subject.starts_with("arrangement/") {
                continue;
            }
            let reduced = match newest {
                Some(newest) => Some(self.cached_subject_status(
                    &connection,
                    &subject,
                    store_index,
                    newest,
                    claims,
                )?),
                None => subject_status_at(
                    &connection,
                    &subject,
                    at_index,
                    selected_owner_run,
                    claims,
                )?,
            };
            let Some((status, action)) = reduced else {
                continue;
            };
            if selected.is_none() && !include_history && status.projection.layer != "current" {
                continue;
            }
            pending_actions.extend(action);
            subjects.push(status);
        }
        Ok(StatusResponse {
            store_index,
            subjects,
            pending_actions,
        })
    }

    fn desired_subjects(&self) -> Result<Vec<DesiredSubject>> {
        smallclaims::touched::note_read(|| "kind:intent.desired".to_owned());
        let connection = self.readers.get();
        let mut statement = connection.prepare(
            "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step FROM desired ORDER BY subject",
        )?;
        let rows = statement.query_map([], desired_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn fault_items(&self, person: Option<&str>) -> Result<Vec<AttentionItemView>> {
        Ok(self
            .fault_snapshot(now_ms())?
            .into_iter()
            .map(|fault| fault.item)
            .filter(|item| person.is_none_or(|person| item.person == person))
            .collect())
    }

    fn fault_snapshot(&self, as_of: u128) -> Result<Vec<FaultView>> {
        let mut items = self.subscription_fault_items(as_of)?;
        let (operational, failures) = self.operational_attention_items_with_failures(None, as_of)?;
        items.extend(operational);
        items.extend(
            self.broken_gate_items_from_failures(None, as_of, failures)?
                .into_iter()
                .filter(|item| !item.person.starts_with("person/")),
        );
        items.extend(self.checkpoint_attention_items(None, as_of)?);
        items.extend(self.signature_attention_items()?);
        let fallback = self.fleet_fault_agent()?;
        self.current_attention(items, as_of)?
            .into_iter()
            .map(|item| {
                Ok(FaultView {
                    owner: self.fault_owner(&item, fallback.as_deref())?,
                    item,
                })
            })
            .collect()
    }

    fn signature_attention_items(&self) -> Result<Vec<AttentionItemView>> {
        let mut items = Vec::new();
        for fault in self.signature_state_faults()? {
            items.push(AttentionItemView {
                episode: format!("{}:{}", fault.code, fault.subject),
                priority: "high".into(),
                kind: "fault".into(),
                review_mode: None,
                subject: fault.subject,
                person: String::new(),
                requester_id: None,
                launch_id: None,
                variant_id: None,
                message_id: None,
                prompt_call: None,
                conversation: None,
                answers: Vec::new(),
                title: fault.code,
                detail: fault.detail,
                request: None,
                mission: None,
                mission_run: None,
                step: None,
                targets: vec![format!("host/{}", self.origin)],
                requested_at_unix_ms: 0,
                actions: vec![attention_action("inspect signature state", &["st", "doctor"])],
            });
        }
        Ok(items)
    }

    fn selected_desired_token(&self, subject: &str) -> Result<Option<String>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        current_desired_row(&connection, subject).map(|row| row.map(|row| row.claim_id))
    }

    fn selected_desired_revision(&self, subject: &str) -> Result<Option<String>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        current_desired_row(&connection, subject).map(|row| row.map(|row| row.revision))
    }
}


/// Whether a current view at `store_index` leaves out runtime `subject`: exactly when its
/// reduction there would place it in history. `owners` says whether a declared runtime's owning
/// run and generation may decide it; the snapshot reduction reads them from their claims. The
/// roster reads many runtimes at once ([`Store::runtime_view_entries`]); this one-runtime read
/// is what its parity tests compare it with.
#[cfg(any(test, feature = "test-support"))]
pub fn runtime_view_entry(
    connection: &Connection,
    subject: &str,
    store_index: u64,
    owners: bool,
) -> Result<ViewEntry> {
    let head = subject_head_at(connection, subject, store_index)?;
    // A folded status can be live only when some claim carries a live status, and a declaration
    // is one of these claims, so a runtime with none of them is stopped or undeclared: history.
    let candidate = connection
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-ha0900c5c496c owner=st3_attention::store TODO=trace-statement-and-value-representation */ &format!(
            "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_current_view_index
                           WHERE subject=?1 AND store_index<=?2 AND {CURRENT_VIEW_CLAIM})"
        ))?
        .query_row(/* sqlite-large-value: allow-for-now lv-h2ad659d96198 owner=st3_attention::store TODO=trace-statement-and-value-representation */ params![subject, store_index], |row| row.get::<_, bool>(0))?;
    runtime_view_entry_from(
        connection,
        subject,
        store_index,
        owners,
        head,
        candidate,
        &mut || desired_row_at(connection, subject, Some(store_index)),
        &mut |of| latest_actual_at(connection, of, Some(store_index)),
        &mut |owner| subject_head_at(connection, owner, store_index),
    )
}
