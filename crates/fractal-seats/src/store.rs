//! The part of `store` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
pub use fractal_missions::store::identities;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::shared::*;
use crate::SeatsStore;
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
pub use runtime_starts::{RuntimeStartAdmission, RuntimeStartOperation, RuntimeStartLock};
pub use roster_admission::AdmissionGuard;
pub use attention_snapshot::{
    CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, native_prompt_gone_key,
    plan_approval, screen_prompt_delivered_key, screen_prompt_gone_key,
    screen_prompt_undelivered_key,
};
pub use fractal_missions::store::mission_eligibility::MISSING_AGENT_CONDITION;
pub use fractal_missions::store::database_size::DatabaseSize;
pub use person_work::AskImages;
pub use fractal_missions::store::checkpoint_rules::{RULES_VERSION, plan_drops, rules_digest};
pub use limits::{AccountLimit, LIMITS_ACTOR, LIMITS_CATCH_UP_PAGE, LimitsOutcome, LimitsPolicy};
pub use limits::ACCOUNT_READING_WINDOW_MS;
pub use subagents::{
    EndedSubagent, SUBAGENT_KINDS, SUBAGENT_LEASE_MS, SubagentSweep, SubagentView,
};
pub use fractal_missions::store::runtime::SmalltalkRuntime;
#[cfg(any(test, feature = "test-support"))]
pub use smallclaims::sqlite::STATEMENTS_RUN;
use crate::resource::newer_comment;
use fractal_kernel::model::LOCAL_OBSERVATION_ID_PREFIX;
pub use fractal_kernel::model::local_observation_position;
pub use crate::incremental::{Change, ChangeFeed};
pub use crate::model::canonical_child_string;
pub use fractal_missions::store::*;
pub use fractal_missions::store::{AgentStatusEntry, AgentWorkQueue, CachedUsageFold, ChildMissionContext, ClaimAdmission, ClaimCommitContext, ClaimsReduction, CumulativeUsage, DesiredRow, DiscoveredItem, GATE_BROKEN_CONDITION, HumanAttention};
pub use fractal_missions::store::{IdempotencyConflict, MAX_STEP_WAIT_MS, MailboxWatermark, MessageCacheEntry, RANGE_SUBJECTS, RECENTLY_ENDED_MS, RUN_REPORT_KIND, StatusEntry, StepLabel, StepTiming, Store, SubjectCache};
pub use fractal_missions::store::{SubjectStatusInputs, UNREPAIRED_CLAIM, USAGE_RESPONSE_HORIZON_MS, UsageFold, UsageFolds, UsageSpend, ViewEntry, WORK_NUDGED_KIND, agent_queue_values, agent_view, analyze_mission_revision, append_claim_fenced_outcome};
pub use fractal_missions::store::{append_claim_tx, append_claim_with_commit_context, arrangements, assistant_write, attention, attention_list, backup, backup_format, card_fold, check_mailbox_fence, checkpoint, checkpoint_rules};
pub use fractal_missions::store::{checkpoint_status, claim_append_error, claim_log_order, client_summary, coordination, custom, database_size, declarations, delivery, desired_from_row, desired_revision, end_watch_tx};
pub use fractal_missions::store::{fold_step_clock, fold_step_timing, format_wait_span, generation_id_from_subject, git_change_tables, github_post_agent_tx, glass_heads, glasses, idle_mail, insert_local_observation_tx, lanes, local_retention};
pub use fractal_missions::store::{loop_run_views_tx, mailbox_changes, mailbox_wakes, message_bodies, migrate_local_usage_seen, mission_eligibility, mission_list, mission_provenance, mission_run_header_tx, mission_run_outcome_tx, mission_run_variables, missions};
pub use fractal_missions::store::{normalize_message_party, observer_actuals, observer_subscription_detail, operation_audit, owned_sets, owner_lists, project_mission_run_created, project_replicated_mission_runs, projection_digest, projection_divergence, projectors, publication_facts};
pub use fractal_missions::store::{published_list, published_views, rebuild_planning_tx, recently_ended_since, replay_graph_from_nothing_tx, resources, run_report_fields, runtime, runtime_list, seat_queue_joins_query, seat_queue_moves_query, select_replicated_desired};
pub use fractal_missions::store::{select_replicated_mission, stale_pull_request_request_tx, step_execution_clock_at, step_run_from_row, step_timeout_episode, step_timeout_extension_at, step_timing_lease_at, subject_status_at, subscription_started_run_tx, summary_list, unread_mail, validate_claim_input};
pub use fractal_missions::store::{validate_local_observation, watch_ended_tx, work_list};
pub mod accounts;
pub mod adhoc_work;
pub mod agent_messages;
pub mod attention_ivm;
pub mod attention_snapshot;
pub mod conversation_reads;
pub mod delegation;
pub mod directive_notes;
pub mod evidence;
pub mod github_workflow_failures;
pub mod harness_questions;
pub mod limits;
pub mod mailbox_lease;
pub mod message_subscriptions;
pub mod mission_ivm;
pub mod operations;
pub mod person_work;
pub mod publication;
pub mod reconcile_work_context;
pub mod resource_preparation;
pub mod retirements;
pub mod revision_seats;
pub mod rollouts;
pub mod roster_admission;
pub mod runtime_start_diagnostics;
pub mod runtime_starts;
pub mod seat_status;
pub mod seats;
pub mod step_labels;
pub mod stop_deferral;
pub mod subagents;
pub mod trace_principal;
pub mod usage_accounts;
pub use usage_accounts::AttributedUsage;
pub mod usage_period;
pub mod work_summaries_ivm;
#[cfg(test)]
pub mod accountable_tests;
#[cfg(test)]
pub mod agents_window_deadline_tests;
#[cfg(test)]
pub mod arrangements_tests;
#[cfg(test)]
pub mod checkpoint_tests;
#[cfg(test)]
pub mod fleet_admission_tests;
#[cfg(test)]
pub mod harness_event_tests;
#[cfg(test)]
pub mod mission_model_tests;
#[cfg(test)]
pub mod native_resume_tests;
#[cfg(test)]
pub mod resource_preparation_fence_tests;
#[cfg(test)]
pub mod tests;
#[cfg(any(test, feature = "test-support"))]
pub mod terminal_gate_evidence;

/// The newest claim at or before a snapshot that an agent status can depend on.
const AGENT_STATUS_INDEX_QUERY: &str = "SELECT store_index FROM claims WHERE store_index<=?1
     AND (+subject GLOB 'agent/*' OR +subject GLOB 'mission-run/*'
          OR +subject GLOB 'run-generation/*')
     ORDER BY store_index DESC LIMIT 1";

const RETIRED_OWNED_INTAKE_SUBJECTS: &str = "SELECT desired.subject
     FROM desired
     JOIN mission_runs owner
       ON substr(desired.owner_run, 1, 12)='mission-run/'
      AND owner.id=substr(desired.owner_run, 13)
     JOIN mission_runs root ON root.id=owner.root_run_id
     WHERE desired.kind IN ('observer','subscription','schedule')
       AND (
         owner.status IN ('completed','failed','cancelled')
         OR owner.phase='terminal'
         OR root.status IN ('completed','failed','cancelled')
         OR root.phase='terminal'
         OR (
           desired.owner_generation IS NOT NULL
           AND desired.owner_generation != ('run-generation/' || owner.current_generation_id)
         )
       )
     ORDER BY desired.subject";

/// The local rows a roster card can read: timeline entries an agent subject made, which date its
/// activity and its managed session. Heartbeat and telemetry observations never change a card —
/// the harness fold, working episodes, usage summaries and todos all read replicated claims —
/// and neither do `Latest` observations, which reach cards only through the claims written when
/// their state changes. So the frontier is the newest agent timeline row at or before the cut,
/// an exact `MAX(id)`: it never assumes observation ids and graph cuts rise together, which
/// repair and trim can break.
const ROSTER_LOCAL_FRONTIER: &str = "SELECT COALESCE((SELECT id FROM local_observations
 WHERE kind='harness.timeline' AND subject LIKE 'agent/%' AND after_store_index<=?1
 ORDER BY id DESC LIMIT 1), 0)";

/// One frontier read, inside the caller's SQLite snapshot when it holds one: the reader pool
/// lends pinned reads the same connection. The partial frontier index keeps an old cut from
/// walking newer heartbeat rows.
pub fn roster_local_frontier(connection: &Connection, index: u64) -> Result<u64> {
    Ok(connection.query_row(/* sqlite-large-value: allow-for-now lv-h6ec3fa9174f4 owner=fractal_seats::store TODO=trace-statement-and-value-representation */ ROSTER_LOCAL_FRONTIER, [index], |row| row.get::<_, u64>(0))?)
}

#[derive(Default)]
pub struct AgentResourcesDelta {
    pub subjects: BTreeSet<String>,
    pub queues: bool,
    pub membership: bool,
}

pub enum AgentResourcesChange {
    Delta(AgentResourcesDelta),
    Invalidated { reason: &'static str, detail: String },
}

/// The exact cache-hit contract shared by the building path and the read-only warm pin: one
/// graph cut, one roster-relevant local frontier, one history mode, a queue lease window that
/// has not ended, and coverage of every requested subject.
fn agent_resources_entry_hits(
    entry: &runtime::AgentResourcesEntry,
    now: u128,
    index: u64,
    local: u64,
    history: bool,
    selected: Option<&BTreeSet<String>>,
) -> bool {
    entry.index == index
        && entry.local == local
        && entry.history == history
        && entry.valid_until_unix_ms.is_none_or(|expiry| now < expiry)
        && match selected {
            None => entry.covered.is_none(),
            Some(names) => entry
                .covered
                .as_ref()
                .is_none_or(|covered| names.is_subset(covered)),
        }
}

pub trait SeatsStoreCore {
    fn apply_internal(
            &self,
            intent: &NormalizedIntent,
            key: &str,
        ) -> Result<ApplyResponse, St3Error>;
    /// Reduce only subjects whose IDs begin with `prefix`.
    ///
    /// Product projections use this instead of reducing every subject in the graph and filtering
    /// afterwards. The latter made small agent and host screens temporarily allocate the complete
    /// claim graph on large stores. `claims` picks what the answers carry: projections that never
    /// serialize `SubjectStatus::claims` pass [`ClaimsReduction::Summary`].
    fn status_for_subject_prefix_at(
            &self,
            prefix: &str,
            at_index: Option<u64>,
            include_history: bool,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse>;
    /// Compare shared claims by canonical order, independent of replica arrival.
    fn claim_is_after(&self, left: &str, right: &str) -> Result<bool>;
    /// The subjects among `subjects` that a person or an agent stopped while `generation` of
    /// `run` declared them. Such a stop holds for the rest of that generation, across daemon
    /// restarts, until someone starts the seat again; the run does not declare it again.
    fn stopped_by_hand(
            &self,
            subjects: &[String],
            run: &str,
            generation: &str,
        ) -> Result<BTreeSet<String>>;
    /// The desired subjects named in `subjects`, by their primary keys.
    fn desired_subjects_named(&self, subjects: &[String]) -> Result<Vec<DesiredSubject>>;
}

impl SeatsStoreCore for Store {

    fn apply_internal(
            &self,
            intent: &NormalizedIntent,
            key: &str,
        ) -> Result<ApplyResponse, St3Error> {
        self.apply_internal_with_declared_agents(intent, key, &BTreeSet::new())
    }

    fn status_for_subject_prefix_at(
            &self,
            prefix: &str,
            at_index: Option<u64>,
            include_history: bool,
            claims: ClaimsReduction,
        ) -> Result<StatusResponse> {
        // Agent listings are expensive on large graphs. Hold this lock while building the
        // snapshot so concurrent callers share one reduction, then serve clones at the same
        // store index. A later index always rebuilds, preserving snapshot semantics.
        if prefix == "agent/" {
            // What this thread's reads can see, as every snapshot read checks: a read pinned to
            // a snapshot can see a commit a moment before the writer publishes its index.
            let current = current_index(&self.readers.get())?;
            let index = selected_index(current, at_index).map_err(anyhow::Error::new)?;
            let mut cache = self
                .smalltalk
                .agent_status_cache
                .lock()
                .expect("agent status cache poisoned");
            if let Some((_, _, _, _, status)) =
                cache.iter().find(|(cached_index, _, history, mode, _)| {
                    *cached_index == index && *history == include_history && *mode == claims
                })
            {
                let mut result = (**status).clone();
                result.store_index = index;
                return Ok(result);
            }
            let projection_index = self.agent_status_index(index)?;
            if let Some((cached_index, _, _, _, status)) = cache
                .iter_mut()
                .find(|(_, cached_projection, history, mode, _)| {
                    *cached_projection == projection_index
                        && *history == include_history
                        && *mode == claims
                })
            {
                *cached_index = index;
                let mut result = (**status).clone();
                result.store_index = index;
                return Ok(result);
            }
            let status = self.status_for_subject_prefix_uncached(
                prefix,
                Some(index),
                include_history,
                claims,
            )?;
            cache.push_back((
                index,
                projection_index,
                include_history,
                claims,
                Arc::new(status.clone()),
            ));
            if cache.len() > 8 {
                cache.pop_front();
            }
            return Ok(status);
        }
        self.status_for_subject_prefix_uncached(prefix, at_index, include_history, claims)
    }

    fn claim_is_after(&self, left: &str, right: &str) -> Result<bool> {
        let connection = self.readers.get();
        Ok(canonical::claim_key(&connection, left)? > canonical::claim_key(&connection, right)?)
    }

    fn stopped_by_hand(
            &self,
            subjects: &[String],
            run: &str,
            generation: &str,
        ) -> Result<BTreeSet<String>> {
        let mut stopped = BTreeSet::new();
        for desired in self.desired_subjects_named(subjects)? {
            // A run's own stop and a generation's retirement record their owner; only a stop
            // published at the root, as `st agents stop` does, is someone's decision.
            if desired.kind != "stop"
                || desired.owner_run.is_some()
                || desired.owner_generation.is_some()
            {
                continue;
            }
            let Some(ended) = self.declaration_ended_by_stop(&desired.subject)? else {
                continue;
            };
            if ended.writer.is_some()
                && ended.declaration.owner_run.as_deref() == Some(run)
                && ended.declaration.owner_generation.as_deref() == Some(generation)
            {
                stopped.insert(desired.subject);
            }
        }
        Ok(stopped)
    }

    fn desired_subjects_named(&self, subjects: &[String]) -> Result<Vec<DesiredSubject>> {
        for subject in subjects {
            smallclaims::touched::note_read(|| subject.clone());
        }
        if subjects.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step
             FROM desired WHERE subject IN (SELECT value FROM json_each(?1)) ORDER BY subject",
        )?;
        let rows = statement.query_map([serde_json::to_string(subjects)?], desired_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}


#[cfg(any(test, feature = "test-support"))]
use fractal_missions::store::shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
pub use fractal_missions::store::shared_seats::{BASE_REBUILDS, DEFINITIONS_PARSED, RUN_PART_RUNS, STATUS_AFTER_DESIRED_READ, STEPS_ENRICHED, SUBJECT_REDUCTIONS};
/// A document republished all day, such as a probe report, arrives from its writer after this
/// node's own newer claims (#898). Each binding keeps the earliest claim of its version whatever
/// order bindings arrive in, so it extends the projection without rebuilding every version, and
/// the result is the one a replay from nothing gives.
#[cfg(any(test, feature = "test-support"))]
#[test]
pub fn a_document_binding_that_arrives_out_of_order_does_not_rebuild_every_version() {
    pub const FLEET: &str = "018f6f0d-4a5d-7b8c-9d0e-123456789abc";
    let source = Store::open_memory("source").unwrap();
    source.bind_fleet(FLEET).unwrap();
    let target = Store::open_memory("target").unwrap();
    target.bind_fleet(FLEET).unwrap();
    let sync = || {
        let response = source
            .export_replication_exchange(FLEET, &target.replication_inventory().unwrap())
            .unwrap();
        target
            .receive_replication_exchange("source", FLEET, &response)
            .unwrap();
        target.validate_replication_backlog().unwrap();
        target.project_replication_backlog().unwrap();
    };
    let publish = |version: usize| {
        source
            .put_document(
                "doc/probe-report",
                format!("An invented probe report, version {version}.").as_bytes(),
                &source.latest_document_token("doc/probe-report").unwrap(),
                &format!("probe-report-{version}"),
            )
            .unwrap();
    };
    for version in 0..40 {
        publish(version);
    }
    sync();

    // The target writes a claim of its own that sorts after the next binding.
    let accepted_at = source
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT MAX(CAST(accepted_at_unix_ms AS INTEGER)) FROM claims",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as u128
        + 10;
    target.set_write_clock_at(accepted_at + 10).unwrap();
    target
        .append_claim(&ClaimInput {
            subject: "custom/target/newer".into(),
            kind: "custom.target.note".into(),
            actor: Some("person/tester".into()),
            fields: BTreeMap::new(),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        })
        .unwrap();
    target.project_replication_backlog().unwrap();
    source.set_write_clock_at(accepted_at).unwrap();
    publish(40);
    FULL_REPLAYS.with(|replays| replays.set(0));
    BASE_REBUILDS.with(|rebuilds| rebuilds.set(0));
    sync();
    assert_eq!(FULL_REPLAYS.with(std::cell::Cell::get), 0);
    assert_eq!(BASE_REBUILDS.with(std::cell::Cell::get), 0);

    let documents = |store: &Store| {
        store
            .connection
            .lock()
            .unwrap()
            .prepare(
                "SELECT name, hash, created_index, binding_claim_id, binding_key FROM documents
                 ORDER BY name, hash",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let incremental = documents(&target);
    assert_eq!(incremental.len(), 41);
    target.replay_replication_graph().unwrap();
    assert_eq!(documents(&target), incremental);
}
