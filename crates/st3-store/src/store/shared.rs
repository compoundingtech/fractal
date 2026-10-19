#![allow(unused_imports)]
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
use smallclaims::replication::{ReplicaRange, ReplicationBatch, ReplicationResponse};
use crate::model::{SeatQueueMoveRequest, SeatQueueMoveView, SeatQueueRunView, SeatQueueView};
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
use crate::store::{AgentStatusEntry, AgentWorkQueue, CachedUsageFold, ChildMissionContext, ClaimAdmission, ClaimCommitContext, ClaimsReduction, CumulativeUsage, DesiredRow};
use crate::store::{GATE_BROKEN_CONDITION, HumanAttention, IdempotencyConflict, MailboxWatermark, MessageCacheEntry, RANGE_SUBJECTS, RECENTLY_ENDED_MS, StatusEntry, StepLabel, StepTiming};
use crate::store::{Store, SubjectCache, SubjectStatusInputs, UNREPAIRED_CLAIM, USAGE_RESPONSE_HORIZON_MS, UsageFold, UsageFolds, UsageSpend, ViewEntry, WORK_NUDGED_KIND, analyze_mission_revision, append_claim_fenced_outcome};
use crate::store::{append_claim_tx, append_claim_with_commit_context, check_mailbox_fence, claim_append_error, claim_log_order, desired_from_row, desired_revision, fold_step_clock, fold_step_timing, generation_id_from_subject};
use crate::store::{insert_local_observation_tx, local_retention, loop_run_views_tx, migrate_local_usage_seen, mission_run_header_tx, mission_run_outcome_tx, mission_run_variables, normalize_message_party, project_mission_run_created, project_replicated_mission_runs, rebuild_planning_tx};
use crate::store::{recently_ended_since, replay_graph_from_nothing_tx, runtime, seat_queue_joins_query, seat_queue_moves_query, select_replicated_desired, select_replicated_mission, stale_pull_request_request_tx, step_execution_clock_at, step_run_from_row};
use crate::store::{step_timeout_episode, step_timeout_extension_at, subject_status_at, subscription_started_run_tx, validate_claim_input, validate_local_observation};
#[cfg(any(test, feature = "test-support"))]
#[cfg(any(test, feature = "test-support"))]
use crate::store::{WORK_ACTION_WRITER_WORK, WorkActionWriterProbe, append_latest_observation, expire_work_lease_by_claim, mailbox_fence_evaluations, mailbox_incarnation_evaluations, message_canonical_key_query, message_canonical_key_tx, named_run_probes, runtime_view_entry, step_execution_timing_at};
#[cfg(any(test, feature = "test-support"))]
use crate::store::{try_project_all_simple_replication_tx, validate_replicated_claim, verify_replica_batch};

#[cfg(any(test, feature = "test-support"))]
impl Store {
    /// Hold every read connection the pool has, as a burst of long reads does, so a read
    /// inside has to open another.
    pub fn hold_read_connections_for_test(&self, hold: impl FnOnce()) {
        let idle_count = self
            .readers
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let guards: Vec<_> = (0..idle_count).map(|_| self.readers.get()).collect();
        hold();
        drop(guards);
    }

    pub fn hold_writer_for_test(&self, hold: impl FnOnce()) {
        let _writer = self.connection.write();
        hold();
    }

    /// Record local observations as `append_claim` does, in one transaction: one commit, and
    /// so one sync to disk, instead of one for each observation.
    pub fn append_local_observations_for_test(&self, inputs: &[ClaimInput]) {
        let mut connection = self.connection.write();
        let transaction = connection.transaction().unwrap();
        for input in inputs {
            self.validate_claim_input(input).unwrap();
            assert!(
                local_retention(&input.kind),
                "{} is not a local observation",
                input.kind
            );
            validate_local_observation(input).unwrap();
            insert_local_observation_tx(&transaction, &self.origin, input, now_ms()).unwrap();
        }
        transaction.commit().unwrap();
    }

    pub fn hold_write_transaction_for_test(&self, hold: impl FnOnce()) {
        let mut writer = self.connection.write();
        let transaction = writer.transaction().unwrap();
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-h5eb1db1a9333 owner=st3_store::store::shared TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key, value) VALUES ('test-write-stall', '1')",
                [],
            )
            .unwrap();
        hold();
        transaction.rollback().unwrap();
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Store {
    #[cfg(any(test, feature = "test-support"))]
    pub fn import_replication(
        &self,
        relay: &str,
        input: &ReplicationBatch,
    ) -> Result<ReplicationResponse, St3Error> {
        if relay != input.peer {
            return Err(St3Error::new(
                "peer-label-mismatch",
                "the configured relay label does not match the batch label",
            ));
        }
        for (hash, bytes) in &input.blobs {
            if hex::encode(Sha256::digest(bytes)) != *hash {
                return Err(St3Error::new(
                    "blob-hash-mismatch",
                    format!("replicated blob `{hash}` failed verification"),
                ));
            }
        }
        let mut connection = self.connection.write();
        let transaction = connection.transaction().map_err(internal)?;
        for (hash, bytes) in &input.blobs {
            transaction
                .execute(
                    /* sqlite-large-value: allow-for-now lv-hbb66800c7218 owner=st3_store::store::shared TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO blobs(hash, bytes, size) VALUES (?1, ?2, ?3)",
                    params![hash, bytes, bytes.len() as u64],
                )
                .map_err(internal)?;
        }
        let mut missing_ranges = Vec::new();
        let mut blocked_origins = BTreeSet::new();
        for batch in &input.batches {
            if blocked_origins.contains(&batch.origin) {
                continue;
            }
            let accepted_through = transaction
                .query_row(
                    "SELECT MAX(replica_sequence) FROM batches WHERE origin=?1",
                    [&batch.origin],
                    |row| row.get::<_, Option<u64>>(0),
                )
                .map_err(internal)?
                .unwrap_or(0);
            if batch.replica_sequence <= accepted_through {
                let stored_hash = transaction
                    .query_row(
                        "SELECT hash FROM batches WHERE origin=?1 AND replica_sequence=?2",
                        params![batch.origin, batch.replica_sequence],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(internal)?;
                if stored_hash.as_deref() != Some(batch.hash.as_str()) {
                    return Err(St3Error::new(
                        "replica-sequence-conflict",
                        format!(
                            "replica `{}` sequence {} has another hash",
                            batch.origin, batch.replica_sequence
                        ),
                    ));
                }
                continue;
            }
            if batch.replica_sequence != accepted_through.saturating_add(1) {
                missing_ranges.push(ReplicaRange {
                    origin: batch.origin.clone(),
                    from: accepted_through.saturating_add(1),
                    through: batch.replica_sequence.saturating_sub(1),
                });
                blocked_origins.insert(batch.origin.clone());
                continue;
            }
            let expected_previous = transaction
                .query_row(
                    "SELECT hash FROM batches WHERE origin=?1 ORDER BY replica_sequence DESC LIMIT 1",
                    [&batch.origin],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(internal)?;
            if expected_previous != batch.previous_hash {
                return Err(St3Error::new(
                    "broken-replica-chain",
                    format!(
                        "replica sequence {} does not cite the current chain head",
                        batch.replica_sequence
                    ),
                ));
            }
            verify_replica_batch(batch)?;
            transaction
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h2c2cff9abb73 owner=st3_store::store::shared TODO=chunk-or-reference-exact-wire-values */ "INSERT OR IGNORE INTO batches(id, origin, replica_sequence, previous_hash, hash, accepted_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![batch.id, batch.origin, batch.replica_sequence, batch.previous_hash, batch.hash, batch.accepted_at_unix_ms.to_string()],
                )
                .map_err(internal)?;
            for claim in &batch.claims {
                ensure_claim_blobs(&transaction, claim)?;
                let inserted = transaction
                    .prepare_cached(
                        /* sqlite-large-value: allow-for-now lv-h5ee472647443 owner=st3_store::store::shared TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO claims(id, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    )
                    .map_err(internal)?
                    .execute(/* sqlite-large-value: allow-for-now lv-h5864f835da0e owner=st3_store::store::shared TODO=trace-statement-and-value-representation */ params![
                        claim.id,
                        claim.batch_id,
                        claim.subject,
                        claim.kind,
                        claim.origin,
                        claim.actor,
                        canonical_json_text(&claim.body).map_err(internal)?,
                        serde_json::to_string(&claim.predecessors).map_err(internal)?,
                        claim.accepted_at_unix_ms.to_string(),
                    ])
                    .map_err(internal)?;
                if inserted != 0 {
                    let index = transaction.last_insert_rowid() as u64;
                    let effective = register_operation_tx(&transaction, claim).map_err(internal)?;
                    if effective {
                        insert_event(
                            &transaction,
                            index,
                            &claim.kind,
                            &claim.subject,
                            &claim.body,
                        )
                        .map_err(internal)?;
                    }
                    if effective && claim.kind == "intent.desired" {
                        if let Ok(desired) =
                            serde_json::from_value::<DesiredSubject>(claim.body.clone())
                        {
                            select_replicated_desired(&transaction, claim, &desired)?;
                        }
                    } else if effective && claim.kind == retirements::KIND {
                        retirements::project_subject(&transaction, &claim.subject)?;
                    } else if effective && claim.kind == "doc.bound" {
                        select_replicated_document(&transaction, claim, index)?;
                    } else if effective && claim.kind == "mission.published" {
                        select_replicated_mission(&transaction, claim, index)?;
                    }
                }
            }
            transaction
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h57f0dab9fb8f owner=st3_store::store::shared TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO peer_replica_cursors(peer, origin, accepted_through) VALUES (?1, ?2, ?3)
                     ON CONFLICT(peer, origin) DO UPDATE SET accepted_through=MAX(accepted_through, excluded.accepted_through)",
                    params![relay, batch.origin, batch.replica_sequence],
                )
                .map_err(internal)?;
        }
        rebuild_operations_tx(&transaction).map_err(internal)?;
        owned_sets::project_tx(&transaction)?;
        project_replicated_mission_runs(&transaction)?;
        rebuild_planning_tx(&transaction).map_err(internal)?;
        custom::flush(&transaction).map_err(internal)?;
        let accepted_heads = replica_heads(&transaction).map_err(internal)?;
        let accepted_through = accepted_heads.get(&input.peer).copied().unwrap_or(0);
        let missing_sequences = missing_ranges
            .iter()
            .filter(|range| range.origin == input.peer)
            .flat_map(|range| range.from..=range.through)
            .collect::<Vec<_>>();
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-he27d6f0f9355 owner=st3_store::store::shared TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO peer_cursors(peer, accepted_through) VALUES (?1, ?2)
                 ON CONFLICT(peer) DO UPDATE SET accepted_through=excluded.accepted_through",
                params![relay, accepted_through],
            )
            .map_err(internal)?;
        transaction.commit().map_err(internal)?;
        Ok(ReplicationResponse {
            accepted_through,
            missing_sequences,
            accepted_heads,
            missing_ranges,
        })
    }
}
