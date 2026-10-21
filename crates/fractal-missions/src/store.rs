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
use crate::MissionsStore;
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
pub use fractal_delivery::store::runtime_starts::RuntimeStartOperation;
pub use attention_snapshot::{
    CLAUDE_PROMPT_ANSWER_WAIT_MS, NativePromptState, PromptAnswer, native_prompt_gone_key,
    plan_approval, screen_prompt_delivered_key, screen_prompt_gone_key,
    screen_prompt_undelivered_key,
};
pub use mission_eligibility::MISSING_AGENT_CONDITION;
pub use fractal_delivery::store::database_size::DatabaseSize;
pub use person_work::AskImages;
pub use fractal_delivery::store::checkpoint_rules::{RULES_VERSION, plan_drops, rules_digest};
pub use fractal_delivery::store::limits::{AccountLimit, LIMITS_CATCH_UP_PAGE};
pub use fractal_delivery::store::limits::ACCOUNT_READING_WINDOW_MS;
pub use fractal_delivery::store::subagents::{SUBAGENT_KINDS, SubagentView};
pub use fractal_delivery::store::runtime::SmalltalkRuntime;
#[cfg(any(test, feature = "test-support"))]
pub use smallclaims::sqlite::STATEMENTS_RUN;
use crate::resource::newer_comment;
use fractal_kernel::model::LOCAL_OBSERVATION_ID_PREFIX;
pub use fractal_kernel::model::local_observation_position;
pub use crate::incremental::{Change, ChangeFeed};
pub use crate::model::canonical_child_string;
pub use fractal_delivery::store::*;
pub use fractal_delivery::store::{ACTUAL_CACHE_LIMIT, AgentStatusEntry, AgentWorkQueue, CachedUsageFold, ChildMissionContext, ClaimAdmission, ClaimCommitContext, ClaimsReduction, CumulativeUsage, DesiredRow, EffectiveStepOwner, GATE_BROKEN_CONDITION};
pub use fractal_delivery::store::{HumanAttention, INCARNATION_OF_CLAIM, IdempotencyConflict, LOCAL_OBSERVATION_COLUMNS, MailboxWatermark, MessageCacheEntry, PinnedDefinitions, RANGE_SUBJECTS, RECENTLY_ENDED_MS, RUNTIME_SUBJECTS, RecordedSeatMove, ReferenceGraph};
pub use fractal_delivery::store::{SeatQueueInputs, StatusEntry, StepLabel, StepTiming, Store, SubjectCache, SubjectStatusInputs, UNREPAIRED_CLAIM, USAGE_BUCKETS, USAGE_RESPONSE_HORIZON_MS, UsageFold, UsageFolds};
pub use fractal_delivery::store::{UsageSpend, ViewEntry, WORK_NUDGED_KIND, accounts, agent_messages, agent_queue_values, agent_view, analyze_mission_revision, append_claim_fenced_outcome, append_claim_tx, append_claim_with_commit_context, append_claim_with_fences};
pub use fractal_delivery::store::{append_latest_observation_publication, append_receipt_claim_tx, apply_effective_step_state, apply_effective_step_state_with_owner, assistant_write, attention, attention_list, backup, backup_format, card_fold, carried_revision_step_paths, check_mailbox_fence};
pub use fractal_delivery::store::{checkpoint, checkpoint_rules, checkpoint_status, claim_append_error, claim_by_id_tx, claim_log_order, classify_replicated_claim_with_registry, client_summary, compatible_step_paths, conversation_reads, current_desired_row, current_desired_row_tx};
pub use fractal_delivery::store::{current_harness_at, custom, database_size, delivery, desired_from_row, desired_lineage_from_tx, desired_revision, enrich_step_summaries_at, evidence, fold_step_clock, fold_step_timing, generation_id_from_subject};
pub use fractal_delivery::store::{git_change_tables, glass_heads, glasses, harness_fold_at, harness_questions, idle_mail, insert_local_observation_tx, intent_leaves_tx, is_failed_terminal, latest_actual, latest_retention, limits};
pub use fractal_delivery::store::{local_observation_dedupe_key, local_observation_from_row, local_retention, loop_run_views_tx, mailbox_changes, mailbox_lease, mailbox_wakes, message_bodies, message_subscriptions, migrate_local_usage_seen, mission_provenance, mission_run_header_tx};
pub use fractal_delivery::store::{mission_run_outcome_tx, mission_run_variables, newest_claims_of_kind_query, normalize_message_party, normalize_seat, observer_actuals, observer_subscription_detail, operation_audit, owned_sets, owner_lists, pricing_identity, project_claim_isolated_tx};
pub use fractal_delivery::store::{project_mission_run_created, project_mission_run_update, project_replicated_mission_runs, projection_digest, projection_divergence, projectors, publication, publication_facts, published_list, published_views, rebuild_planning_tx, recently_ended_since};
pub use fractal_delivery::store::{replay_graph_from_nothing_tx, resources, retirements, revision_seats, rollouts, roster_admission, runtime, runtime_list, runtime_starts, same_declared_launch, schema_fields_for_body, seat_queue_inputs_tx};
pub use fractal_delivery::store::{seat_queue_joins_query, seat_queue_moves_query, seat_status, seat_work_at_snapshot_query, seats, select_replicated_desired, select_replicated_mission, stale_pull_request_request_tx, step_execution_clock_at, step_labels, step_run_from_row, step_timeout_episode};
pub use fractal_delivery::store::{step_timeout_extension_at, stop_deferral, subagents, subject_status_at, subscription_started_run_tx, summary_list, system_local_retention, trace_principal, unread_mail, usage_native_session, usage_slot, validate_claim_input};
pub use fractal_delivery::store::{validate_local_observation};
pub mod adhoc_work;
pub mod arrangements;
pub mod attention_ivm;
pub mod attention_snapshot;
pub mod coordination;
pub mod declarations;
pub mod delegation;
pub mod directive_notes;
pub mod github_workflow_failures;
pub mod lanes;
pub mod mission_eligibility;
pub mod mission_ivm;
pub mod mission_list;
pub mod missions;
pub mod operations;
pub mod person_work;
pub mod reconcile_work_context;
pub mod resource_preparation;
pub mod usage_period;
pub mod work_list;
pub mod work_summaries_ivm;
#[cfg(test)]
pub mod arrangements_tests;
#[cfg(test)]
pub mod authored_pull_requests_tests;
#[cfg(test)]
pub mod card_fold_tests;
#[cfg(test)]
pub mod checkpoint_tests;
#[cfg(test)]
pub mod convergence;
#[cfg(any(test, feature = "test-support"))]
pub mod delegation_tests_shared_seats;
#[cfg(test)]
pub mod fleet_admission_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod mission_ivm_tests_shared_seats;
#[cfg(test)]
pub mod mission_model_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod operations_tests_shared_seats;
#[cfg(test)]
pub mod provenance_tests;
#[cfg(test)]
pub mod schedule_active_run_tests;
#[cfg(test)]
pub mod step_wait_tests;
#[cfg(test)]
pub mod tests;
#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_seats;
#[cfg(test)]
pub mod work_renew_tests;
#[cfg(test)]
pub mod writer_bound_tests;

/// The longest one `work wait` stops the execution clock. A wait past it must be declared again,
/// so a seat that stopped answering cannot hide behind a wait.
pub const MAX_STEP_WAIT_MS: u64 = 12 * 60 * 60 * 1000;

/// A wait span as hours and minutes, for the budget error and the step view.
pub fn format_wait_span(milliseconds: u128) -> String {
    let minutes = milliseconds / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h{minutes:02}m"),
    }
}

/// The first (`ASC`) or last (`DESC`) acceptance time of the claims about subject `?1` that
/// name incarnation `?2`, at or before store index `?3`: one seek into the incarnation's claims.
pub fn incarnation_claim_time_query(order: &str) -> String {
    format!(
        "SELECT accepted_at_unix_ms
         FROM claims INDEXED BY claims_incarnation_accepted_index
         WHERE subject=?1 AND {INCARNATION_OF_CLAIM}=?2 AND store_index<=?3
         ORDER BY length(accepted_at_unix_ms) {order}, accepted_at_unix_ms {order}
         LIMIT 1"
    )
}

// Owner joins compare the run ID with the reference's suffix, so SQLite looks each owner up by
// primary key instead of scanning every run for each desired row on every reconcile pass.
pub const TERMINAL_OWNED_RUNTIME_SUBJECTS: &str = "SELECT desired.subject
     FROM desired
     JOIN mission_runs owner
       ON substr(desired.owner_run, 1, 12)='mission-run/'
      AND owner.id=substr(desired.owner_run, 13)
     JOIN mission_runs root ON root.id=owner.root_run_id
     LEFT JOIN run_generations generation
       ON substr(desired.owner_generation, 1, 15)='run-generation/'
      AND generation.id=substr(desired.owner_generation, 16)
     LEFT JOIN step_runs step ON step.subject=desired.owner_step
     WHERE desired.member IS NOT NULL
       AND (
         owner.status IN ('completed','failed','cancelled')
         OR owner.phase='terminal'
         OR root.status IN ('completed','failed','cancelled')
         OR root.phase='terminal'
         OR generation.status IN ('completed','failed','cancelled')
         OR step.status='cancelled'
         OR (owner.phase='final-cancelled' AND desired.kind IN ('exec','pty')
             AND (desired.owner_step IS NULL OR step.status IN ('completed','failed')))
       )
     ORDER BY desired.subject";

/// One repository item whose recorded facts changed, with the facts that changed. `deliver` says
/// whether the change asks a subscription for a review or a triage.
pub struct DiscoveredItem {
    subject: String,
    kind: &'static str,
    facts: Value,
    changed_fields: Vec<String>,
    /// The data types the change belongs to: the item's collection, `comments`, `reactions` or
    /// `mentions`.
    data_types: BTreeSet<String>,
    deliver: bool,
}

/// The data types a change to an item's facts belongs to. A comment, a reaction and a mention
/// each have their own; every other fact belongs to the item's collection.
fn item_data_types(
    collection: &str,
    prior: Option<&Value>,
    facts: &Value,
    changed_fields: &[String],
) -> BTreeSet<String> {
    let mut data_types = BTreeSet::new();
    for field in changed_fields {
        match field.as_str() {
            "comments" => data_types.insert("comments".to_owned()),
            "reactions" => data_types.insert("reactions".to_owned()),
            "mentions" => data_types.insert("mentions".to_owned()),
            "recent_comments" => data_types.insert("comments".to_owned()),
            "last_comment" => {
                let reactions = |value: Option<&Value>| {
                    value
                        .and_then(|value| value.pointer("/last_comment/reactions"))
                        .cloned()
                };
                if reactions(prior) != reactions(Some(facts)) {
                    data_types.insert("reactions".to_owned());
                }
                let without_reactions = |value: Option<&Value>| {
                    let mut comment = value.and_then(|value| value.get("last_comment")).cloned();
                    if let Some(Value::Object(comment)) = comment.as_mut() {
                        comment.remove("reactions");
                    }
                    comment
                };
                if without_reactions(prior) != without_reactions(Some(facts)) {
                    data_types.insert("comments".to_owned())
                } else {
                    false
                }
            }
            _ => data_types.insert(collection.to_owned()),
        };
    }
    data_types
}

/// A pull request is reviewed once for each head it is ready at: when it first appears open and
/// ready, when a draft becomes ready, and when a new head replaces one that was known. Nothing
/// else starts a review. A closure, a merge, a reopening at the same head, a title edit, and a
/// field that an older build did not record never do, because st would review a head it already
/// saw or one nobody can act on. A known item with no known head proves nothing, so a head first
/// recorded for it is a baseline.
fn pull_request_needs_review(prior: Option<&Value>, item: &Value) -> bool {
    // An item resource names the head `head_sha`; a listing names it `head`.
    let head_of = |item: &Value| {
        item.get("head_sha")
            .or_else(|| item.get("head"))
            .filter(|head| !head.is_null())
            .cloned()
    };
    let head = head_of(item);
    let open = item
        .get("state")
        .and_then(Value::as_str)
        .is_none_or(|state| state == "open");
    let ready = item.get("draft").and_then(Value::as_bool) != Some(true);
    if head.is_none() || !open || !ready {
        return false;
    }
    let Some(prior) = prior else {
        return true;
    };
    let prior_head = head_of(prior);
    prior.get("draft").and_then(Value::as_bool) == Some(true)
        || prior_head.is_some_and(|prior_head| Some(prior_head) != head)
}

/// The repository collections whose items become their own resources, with each item's subject
/// segment and resource kind.
const REPOSITORY_COLLECTIONS: [(&str, &str, &str); 2] = [
    ("pull_requests", "pull-request", "vcs.pull-request"),
    ("issues", "issue", "vcs.issue"),
];

/// The most mentions one item resource keeps, newest first.
const ITEM_MENTIONS: usize = 50;

/// What an item resource records after one sighting: its prior facts with every fact the
/// sighting read laid over them. A listing names the head commit `head`; the resource calls it
/// `head_sha`. A fact the sighting read as null is gone, except an unknown head. Only an open
/// pull request is known to be unmerged: one that closes keeps `merged` only when the sighting
/// could tell a merge from a closure. An edit to an older comment does not replace the last
/// comment, and mentions add to the ones known, keeping the newest of each login.
fn item_facts(
    repository: &str,
    field: &str,
    number: u64,
    prior: Option<&Value>,
    seen: &Value,
) -> serde_json::Map<String, Value> {
    let mut facts = prior
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (name, value) in seen.as_object().into_iter().flatten() {
        let name = match name.as_str() {
            "new" => continue,
            "head" => "head_sha",
            name => name,
        };
        match (name, facts.get(name)) {
            (_, _) if value.is_null() => {
                if name != "head_sha" {
                    facts.remove(name);
                }
            }
            ("last_comment", Some(known)) if !newer_comment(value, known) => {}
            // Each comment and review once, by kind and ID; the newest are kept.
            ("recent_comments", known) => {
                let merged = crate::resource::merge_recent_comments(
                    known.and_then(Value::as_array).into_iter().flatten(),
                    value.as_array().into_iter().flatten(),
                    Some(crate::resource::RECENT_COMMENTS),
                );
                facts.insert(name.into(), Value::Array(merged));
            }
            ("mentions", Some(Value::Array(known))) => {
                let mut mentions = BTreeMap::<String, Value>::new();
                for mention in known.iter().chain(value.as_array().into_iter().flatten()) {
                    let Some(login) = mention.get("login").and_then(Value::as_str) else {
                        continue;
                    };
                    let at = |mention: &Value| {
                        mention.get("at").and_then(Value::as_str).map(str::to_owned)
                    };
                    if mentions
                        .get(login)
                        .is_none_or(|kept| at(mention) >= at(kept))
                    {
                        mentions.insert(login.to_owned(), mention.clone());
                    }
                }
                let mut mentions = mentions.into_values().collect::<Vec<_>>();
                mentions.sort_by_key(|mention| {
                    std::cmp::Reverse(mention.get("at").and_then(Value::as_str).map(str::to_owned))
                });
                mentions.truncate(ITEM_MENTIONS);
                mentions.sort_by_key(|mention| {
                    mention
                        .get("login")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
                facts.insert(name.into(), Value::Array(mentions));
            }
            _ => {
                facts.insert(name.into(), value.clone());
            }
        }
    }
    facts.insert("repository".into(), Value::String(repository.into()));
    facts.insert("number".into(), Value::from(number));
    let state = facts
        .entry("state")
        .or_insert_with(|| Value::String("open".into()))
        .clone();
    if field == "pull_requests" {
        facts.entry("draft").or_insert_with(|| Value::Bool(false));
        if state == "open" {
            facts.insert("merged".into(), Value::Bool(false));
        } else if seen.get("merged").is_none() && seen.get("state").is_some() {
            facts.remove("merged");
        }
    }
    facts
}

/// The items of one repository collection whose recorded facts change with this observation.
/// `current` names only the items the observation saw, each with only the facts it read; an
/// item it did not see keeps its facts. `recorded` reads an item resource's latest facts.
///
/// An item resource that was never recorded falls back to the item in the previous repository
/// facts, where builds before item resources kept every item, and a recorded resource takes
/// what it lacks from there; such an item is recorded once even when nothing changed, so its
/// resource holds every fact from then on. An item known to neither is new,
/// unless the observation says it existed before the observer last looked (`"new": false`) or
/// this is the repository's baseline. Only a new item, or a pull request at a new ready head,
/// asks a subscription for a review or a triage.
fn discovered_collection_items(
    repository: &str,
    field: &str,
    previous: Option<&Value>,
    current: &Value,
    recorded: &mut dyn FnMut(&str) -> Option<Value>,
) -> Vec<DiscoveredItem> {
    let Some((_, segment, kind)) = REPOSITORY_COLLECTIONS
        .iter()
        .copied()
        .find(|(name, _, _)| *name == field)
    else {
        return Vec::new();
    };
    // The first observation is the baseline. The first listing that names its repository ID is
    // also the first complete listing: the earlier facts came from a first-page read, so the items
    // this listing adds were missed, not opened. Either way its items become known without a
    // delivery.
    let baseline = previous.is_none()
        || (current.get("repository_id").is_some()
            && previous
                .and_then(|value| value.get("repository_id"))
                .is_none());
    let Some(current_items) = current.get(field).and_then(Value::as_array) else {
        return Vec::new();
    };
    let legacy_items = previous
        .and_then(|value| value.get(field))
        .and_then(Value::as_array);
    current_items
        .iter()
        .filter_map(|item| {
            let number = item.get("number")?.as_u64()?;
            let subject = format!("{repository}/{segment}/{number}");
            let resource = recorded(&subject);
            let legacy = legacy_items
                .and_then(|items| {
                    items
                        .iter()
                        .find(|old| old.get("number").and_then(Value::as_u64) == Some(number))
                })
                .map(|old| Value::Object(item_facts(repository, field, number, None, old)));
            // A resource recorded before items carried `head_sha` takes its head from the listing.
            let prior = match (&resource, legacy) {
                (Some(resource), Some(Value::Object(mut legacy))) => {
                    legacy.extend(resource.as_object().cloned().unwrap_or_default());
                    Some(Value::Object(legacy))
                }
                (Some(resource), _) => Some(resource.clone()),
                (None, legacy) => legacy,
            };
            let prior = prior.as_ref();
            let facts = Value::Object(item_facts(repository, field, number, prior, item));
            if resource.as_ref() == Some(&facts) {
                return None;
            }
            // Creation receipts establish identity and attribution, not observer state.
            let observed_prior = prior.filter(|prior| prior.get("state").is_some());
            let new_item =
                observed_prior.is_none() && !baseline && item.get("new") != Some(&Value::Bool(false));
            let deliver = match (field, observed_prior) {
                ("pull_requests", Some(prior)) => pull_request_needs_review(Some(prior), &facts),
                (_, Some(_)) => false,
                ("pull_requests", None) => new_item && pull_request_needs_review(None, &facts),
                _ => new_item,
            };
            let changed_fields = facts
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, value)| {
                    prior.and_then(|prior| prior.get(name.as_str())) != Some(*value)
                })
                .map(|(name, _)| name.clone())
                .chain(
                    prior
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(|prior| {
                            prior
                                .keys()
                                .filter(|name| facts.get(name.as_str()).is_none())
                                .cloned()
                        }),
                )
                .collect::<Vec<_>>();
            Some(DiscoveredItem {
                subject,
                kind,
                data_types: item_data_types(field, prior, &facts, &changed_fields),
                changed_fields,
                facts,
                deliver,
            })
        })
        .collect()
}

/// How a watch ended, when it did: the `subscription.watch-ended` claim for the watch that began
/// at its `since`. Watching again begins a new watch.
pub fn watch_ended_tx(
    connection: &Connection,
    subject: &str,
    watch: &crate::model::WatchSpec,
) -> Result<Option<Value>, St3Error> {
    smallclaims::touched::note_read(|| subject.to_owned());
    let since = watch.since_unix_ms.to_string();
    let mut statement = connection
        .prepare_cached(
            "SELECT body FROM claims WHERE subject=?1 AND kind='subscription.watch-ended'",
        )
        .map_err(internal)?;
    let bodies = statement
        .query_map([subject], |row| row.get::<_, String>(0))
        .map_err(internal)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(internal)?;
    Ok(bodies
        .into_iter()
        .filter_map(|body| serde_json::from_str::<Value>(&body).ok())
        .map(|body| body["fields"].clone())
        .find(|fields| fields.get("since_unix_ms").and_then(Value::as_str) == Some(since.as_str())))
}

/// End a watch: why, and the final wake that told its seat, if one did. Ending it twice records
/// nothing more.
pub fn end_watch_tx(
    transaction: &rusqlite::Transaction<'_>,
    origin: &str,
    batch_id: Option<&str>,
    subject: &str,
    watch: &crate::model::WatchSpec,
    reason: &str,
    message: Option<&str>,
) -> Result<bool, St3Error> {
    if watch_ended_tx(transaction, subject, watch)?.is_some() {
        return Ok(false);
    }
    let mut fields = json!({
        "reason": reason,
        "since_unix_ms": watch.since_unix_ms.to_string(),
    });
    if let Some(message) = message {
        fields["message"] = Value::String(message.to_owned());
    }
    append_claim_tx(
        transaction,
        origin,
        subject,
        "subscription.watch-ended",
        None,
        &json!({"fields": fields}),
        &[],
        batch_id,
    )
    .map_err(claim_append_error)?;
    Ok(true)
}

/// The seat that registered the comment or review `kind` `id` of a repository.
pub fn github_post_agent_tx(
    connection: &Connection,
    locator: &str,
    kind: &str,
    id: u64,
) -> Result<Option<String>, St3Error> {
    let subject = crate::github_watch::github_post_subject(locator, kind, id);
    let body = connection
        .prepare_cached("SELECT body FROM claims WHERE subject=?1 AND kind='github.posted' LIMIT 1")
        .map_err(internal)?
        .query_row(/* sqlite-large-value: allow-for-now lv-hfbbc8260ab79 owner=fractal_missions::store TODO=trace-statement-and-value-representation */ [&subject], |row| row.get::<_, String>(0))
        .optional()
        .map_err(internal)?;
    Ok(body
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .and_then(|body| body["fields"]["agent"].as_str().map(str::to_owned)))
}

/// The mission runs that published a `resource/mission-run/RUN/pull-request` naming this pull
/// request, by URL or by repository and number, at the same head when both name one. An
/// authoring mission publishes that resource when it opens the pull request.
const AUTHORING_PULL_REQUEST_RUNS_QUERY: &str =
    "SELECT subject, body FROM claims WHERE id IN (
         SELECT id FROM claims WHERE kind='resource.observed'
           AND subject LIKE 'resource/mission-run/%/pull-request'
           AND json_extract(body, '$.fields.facts.url')=?1
         UNION
         SELECT id FROM claims WHERE kind='resource.observed'
           AND subject LIKE 'resource/mission-run/%/pull-request'
           AND json_extract(body, '$.fields.facts.repository')=?2
           AND json_extract(body, '$.fields.facts.number')=?3
     ) ORDER BY CANONICAL_ASC(claims)";

fn authoring_pull_request_runs_tx(
    connection: &Connection,
    facts: &Value,
) -> rusqlite::Result<Vec<String>> {
    let url = facts.get("url").and_then(Value::as_str);
    let repository = facts.get("repository").and_then(Value::as_str);
    let number = facts.get("number").and_then(Value::as_u64);
    if url.is_none() && (repository.is_none() || number.is_none()) {
        return Ok(Vec::new());
    }
    let head = facts.get("head_sha").and_then(Value::as_str);
    // An OR can scan the whole partial index. UNION locates matching claim IDs with one
    // expression-index seek per identity, deduplicates dual matches, then orders only those
    // claims with the same canonical order as the original query.
    let mut statement =
        connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h7e9c49567a6f owner=fractal_missions::store TODO=trace-statement-and-value-representation */ &canonical_sql(AUTHORING_PULL_REQUEST_RUNS_QUERY))?;
    let candidates = statement
        .query_map(params![url, repository, number], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut runs = Vec::new();
    for (resource, body) in candidates {
        let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let authored_head = value
            .pointer("/fields/facts/head_sha")
            .and_then(Value::as_str);
        if head.is_some() && authored_head.is_some() && head != authored_head {
            continue;
        }
        let Some(run) = resource
            .strip_prefix("resource/")
            .and_then(|name| name.strip_suffix("/pull-request"))
        else {
            continue;
        };
        if !runs.iter().any(|known| known == run) {
            runs.push(run.to_owned());
        }
    }
    Ok(runs)
}

/// The claim that changes who a run reports to after it started. Its latest one replaces what
/// the creation claim recorded; one with no `report_to` means the run reports to nobody.
pub const RUN_REPORT_KIND: &str = "mission-run.report-to";

/// The agent, stall limit and completion report that a report claim or a creation claim
/// records, or `None` when it names no agent.
pub fn run_report_fields(body: &Value) -> Option<(String, u64, bool, bool)> {
    let fields = body.get("fields").unwrap_or(body);
    Some((
        fields.get("report_to")?.as_str()?.to_owned(),
        fields
            .get("stalled_after_ms")
            .and_then(Value::as_u64)
            .unwrap_or(crate::mission::DEFAULT_STALLED_AFTER_MS),
        fields
            .get("report_completed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        fields
            .get("report_stalls")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    ))
}

/// The lease expiry of a step's newest replicated `work.claimed`, `work.renewed` or
/// `work.progress` claim. Each kind's newest accepted time is one seek of the subject-kind
/// index; only the claims accepted in those milliseconds are put in canonical order, so the read
/// stays the same size however many reports and renewals the step has.
fn last_replicated_lease_query() -> String {
    let newest = |kind: &str| {
        format!(
            "SELECT '{kind}', (SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_subject_kind_accepted_index
                 WHERE subject=?1 AND kind='{kind}'
                 ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1)"
        )
    };
    canonical_sql(&format!(
        "WITH newest(kind, accepted) AS ({} UNION ALL {} UNION ALL {})
         SELECT json_extract(claims.body, '$.fields.claim_expires_at_unix_ms')
         FROM newest CROSS JOIN claims INDEXED BY claims_subject_kind_accepted_index
           ON claims.subject=?1 AND claims.kind=newest.kind
          AND length(claims.accepted_at_unix_ms)=length(newest.accepted)
          AND claims.accepted_at_unix_ms=newest.accepted
         ORDER BY CANONICAL_DESC(claims) LIMIT 1",
        newest("work.claimed"),
        newest("work.renewed"),
        newest("work.progress"),
    ))
}

fn working_episode_query() -> String {
    format!(
        "SELECT claims.id, json_extract(claims.body, '$.fields.state'), claims.accepted_at_unix_ms
         FROM claims INDEXED BY claims_incarnation_accepted_index
         JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 AND claims.kind='harness.observed'
           AND {INCARNATION_OF_CLAIM}=?2
           AND json_extract(claims.body, '$.fields.incarnation_id')=?2
           AND +claims.store_index<=?3
         ORDER BY length(claims.accepted_at_unix_ms) DESC, claims.accepted_at_unix_ms DESC"
    )
}

fn agent_working_since_at(
    connection: &Connection,
    agent: &str,
    incarnation: &str,
    snapshot_index: u64,
) -> Result<Option<u128>> {
    // Stream accepted-time groups backwards. Resolve complete canonical keys only for
    // ties, before applying the state fold. Non-working ends the episode; null does not.
    // Tie lookups must share the stream's cut, including for standalone callers.
    let snapshot = if connection.is_autocommit() {
        Some(connection.unchecked_transaction()?)
    } else {
        None
    };
    let since = (|| -> Result<Option<String>> {
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h7857e1702237 owner=fractal_missions::store TODO=trace-statement-and-value-representation */ &working_episode_query())?;
        let mut rows = statement.query_map(params![agent, incarnation, snapshot_index], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut group = Vec::<(String, Option<String>, String)>::new();
        let mut since = None;
        let fold_group = |group: &mut Vec<(String, Option<String>, String)>,
                          since: &mut Option<String>|
         -> Result<bool> {
            if group.len() > 1 {
                let mut keyed = group
                    .drain(..)
                    .map(|row| Ok((canonical::claim_key(connection, &row.0)?, row)))
                    .collect::<Result<Vec<_>>>()?;
                keyed.sort_by(|a, b| b.0.cmp(&a.0));
                group.extend(keyed.into_iter().map(|(_, row)| row));
            }
            for (_, state, time) in group.drain(..) {
                match state.as_deref() {
                    Some("working") => *since = Some(time),
                    Some(_) => return Ok(true),
                    None => {}
                }
            }
            Ok(false)
        };
        for row in &mut rows {
            smallclaims::read_budget::check()?;
            let row = row?;
            if group.last().is_some_and(|previous| previous.2 != row.2)
                && fold_group(&mut group, &mut since)?
            {
                return Ok(since);
            }
            group.push(row);
        }
        fold_group(&mut group, &mut since)?;
        Ok(since)
    })()?;
    if let Some(snapshot) = snapshot {
        snapshot.commit()?;
    }
    Ok(since.and_then(|time| time.parse().ok()))
}

/// The current generation's steps of actor `?1` (every actor when null), with finished and
/// pending ones when `?2`, and agentless ones when `?3` and no actor is named. With `open_only`
/// the finished steps are left out in SQL, word for word the predicate of
/// `step_runs_open_index`, so the read covers the fleet's open steps and not every step it ran.
pub fn work_at_snapshot_query(open_only: bool) -> String {
    work_at_snapshot_query_of(open_only, "")
}

/// [`work_at_snapshot_query`] of only the step subjects in the JSON array `?4`, by their key.
pub fn selected_work_at_snapshot_query(open_only: bool) -> String {
    work_at_snapshot_query_of(open_only, "subject IN (SELECT value FROM json_each(?4)) AND")
}

fn work_at_snapshot_query_of(open_only: bool, selected: &str) -> String {
    let open = if open_only {
        "AND status NOT IN ('completed','failed','cancelled')"
    } else {
        ""
    };
    format!(
        "SELECT subject, run_id, step_path, definition_hash, status, attempt, assignee, available_to, agentless, title, goals, worker_reported,
                lease_owner, lease_incarnation, lease_expires_at_unix_ms, blocked_reason, not_before_unix_ms, created_at_unix_ms, updated_at_unix_ms, readiness_epoch, constraints
         FROM step_runs
         WHERE {selected} (agentless=0 OR (?3 AND ?1 IS NULL))
           AND generation_id=(SELECT current_generation_id FROM mission_runs WHERE id=step_runs.run_id)
           AND (?1 IS NULL
                OR assignee=?1
                OR lease_owner=?1
                OR EXISTS (SELECT 1 FROM json_each(step_runs.available_to) WHERE value=?1))
           AND (?2 OR status NOT IN ('pending','completed','failed','cancelled'))
           {open}
         ORDER BY created_at_unix_ms, step_path"
    )
}

/// A reopened run keeps its completed normal work and retries its failed steps. Work that the
/// failure cancelled and every final step start again.
pub fn retain_reopened_step_paths(
    mission: &MissionSpec,
    current: &[StepRunView],
    compatible: &mut BTreeSet<String>,
) {
    let finals = flatten_mission_step_specs(mission)
        .into_iter()
        .filter(|step| step.finally)
        .map(|step| step.path.as_str())
        .collect::<BTreeSet<_>>();
    compatible.retain(|path| {
        current.iter().any(|step| {
            &step.step == path
                && (step.status == "failed"
                    || (step.status == "completed" && !finals.contains(path.as_str())))
        })
    });
}

/// Return the current-generation step that took the place of a superseded step, whoever holds it.
fn current_generation_successor_tx(
    connection: &Connection,
    step: &StepRunView,
) -> rusqlite::Result<Option<StepRunView>> {
    let run_id = step.run.strip_prefix("mission-run/").unwrap_or(&step.run);
    connection
        .query_row(
            "SELECT subject, run_id, step_path, definition_hash, status, attempt, assignee, available_to, agentless, title, goals, worker_reported,
                    lease_owner, lease_incarnation, lease_expires_at_unix_ms, blocked_reason, not_before_unix_ms, created_at_unix_ms, updated_at_unix_ms, readiness_epoch, constraints
             FROM step_runs
             WHERE run_id=?1 AND step_path=?2 AND generation_id<>?3
               AND generation_id=(SELECT current_generation_id FROM mission_runs WHERE id=?1)",
            params![run_id, step.step, generation_id_from_subject(&step.generation)],
            step_run_from_row,
        )
        .optional()
}

fn enrich_step_queue_for_reconcile_cached(
    connection: &Connection,
    view: &mut StepRunView,
    snapshot_unix_ms: u128,
    definitions: &mut PinnedDefinitions,
) -> rusqlite::Result<()> {
    apply_effective_step_state(connection, view, snapshot_unix_ms)?;
    enrich_step_definition_cached(connection, view, definitions)?;
    adhoc_work::enrich_handoff(connection, view, snapshot_unix_ms)?;
    person_work::enrich_responses(connection, view)
}

pub fn enrich_step_definition(connection: &Connection, view: &mut StepRunView) -> rusqlite::Result<()> {
    enrich_step_definition_cached(connection, view, &mut PinnedDefinitions::default())
}

/// The lease that bounds the open execution interval of `subject`'s `attempt` as of
/// `snapshot_unix_ms`, as its claims last named it, when an interval is open under one. A
/// quiet renewal moves the step's lease without a claim, never this one.
pub fn step_timing_lease_at(
    connection: &Connection,
    subject: &str,
    attempt: u32,
    snapshot_unix_ms: u128,
) -> rusqlite::Result<Option<u128>> {
    let (started, _, lease) =
        fold_step_timing_open(&step_timing_events(connection, subject)?, attempt, snapshot_unix_ms);
    Ok(started.and(lease))
}

/// [`fold_step_timing`]'s events folded up to `snapshot_unix_ms`: the open interval's start,
/// if any, the time elapsed in closed intervals, and the lease the claims last named.
fn fold_step_timing_open(
    events: &[(String, Value, u128)],
    attempt: u32,
    snapshot_unix_ms: u128,
) -> (Option<u128>, u128, Option<u128>) {
    let fold = fold_step_clock_open(events, attempt, snapshot_unix_ms);
    (fold.started, fold.elapsed, fold.lease_expires)
}

/// The headers of the runs still running, standing or blocked, read through the open runs index.
pub fn open_mission_run_headers_query() -> String {
    MISSION_RUN_HEADER_SELECT.replace(
        "ORDER BY mission_runs.created_at_unix_ms, mission_runs.id",
        &format!(
            "WHERE {OPEN_MISSION_RUN}
               AND mission_runs.status IN ('running','standing','blocked')
             ORDER BY mission_runs.created_at_unix_ms, mission_runs.id"
        ),
    )
}

pub trait MissionsStoreCore {
    fn cached_idempotency_response<T: DeserializeOwned>(
            &self,
            key: &str,
        ) -> Result<Option<T>>;
    fn apply_drained_revision(
            &self,
            run: &str,
        ) -> Result<Option<RevisionSubmissionView>, St3Error>;
    /// Every reference in a publication that resolves neither in the publication nor here: a
    /// pinned mission revision that is not stored, or a mission, agent or observer that nothing
    /// declares. The publication routes refuse them, and their previews list them as blockers.
    fn unresolved_references(
            &self,
            intent: &NormalizedIntent,
        ) -> Result<Vec<String>, St3Error>;
    /// Drop the history an agreed checkpoint at `cut` would drop on this node.
    #[cfg(any(test, feature = "test-support"))]
    fn trim_checkpoint_for_test(&self, cut: u128) -> usize;
}

impl MissionsStoreCore for Store {

    fn cached_idempotency_response<T: DeserializeOwned>(
            &self,
            key: &str,
        ) -> Result<Option<T>> {
        smallclaims::store::idempotency::cached_response(&self.readers.get(), key)?
            .map(|response| serde_json::from_str(&response))
            .transpose()
            .map_err(Into::into)
    }

    fn apply_drained_revision(
            &self,
            run: &str,
        ) -> Result<Option<RevisionSubmissionView>, St3Error> {
        let run_id = run.strip_prefix("mission-run/").unwrap_or(run);
        let proposal = {
            let connection = self.connection.write();
            let active: u32 = connection
                .query_row(
                    "SELECT COUNT(*) FROM step_runs
                     WHERE generation_id=(SELECT current_generation_id FROM mission_runs WHERE id=?1)
                       AND status IN ('claimed','working','verifying')",
                    [run_id],
                    |row| row.get(0),
                )
                .map_err(internal)?;
            if active != 0 {
                return Ok(None);
            }
            let current_generation: String = connection
                .query_row(
                    "SELECT current_generation_id FROM mission_runs WHERE id=?1",
                    [run_id],
                    |row| row.get(0),
                )
                .map_err(internal)?;
            for descendant in
                descendant_mission_run_ids_tx(&connection, &current_generation).map_err(internal)?
            {
                let active: u32 = connection
                    .query_row(
                        "SELECT COUNT(*) FROM step_runs
                         WHERE run_id=?1
                           AND generation_id=(SELECT current_generation_id FROM mission_runs WHERE id=?1)
                           AND status IN ('claimed','working','verifying')",
                        [descendant],
                        |row| row.get(0),
                    )
                    .map_err(internal)?;
                if active != 0 {
                    return Ok(None);
                }
            }
            let proposal_id = connection
                .query_row(
                    /* sqlite-large-value: allow-for-now lv-hfcf012d9a98a owner=fractal_missions::store TODO=trace-statement-and-value-representation */ &canonical_sql(
                        "SELECT p.id FROM revision_proposals p
                     JOIN mission_runs r ON r.id=p.run_id
                     JOIN claims created ON created.subject='revision-proposal/' || p.id
                       AND created.kind='revision-proposal.created'
                     WHERE p.run_id=?1 AND p.source_generation_id=r.current_generation_id
                       AND p.status='draining'
                     ORDER BY CANONICAL_ASC(created) LIMIT 1",
                    ),
                    [run_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(internal)?;
            proposal_id
                .map(|id| revision_proposal_view_tx(&connection, &id))
                .transpose()
                .map_err(internal)?
        };
        let Some(proposal) = proposal else {
            return Ok(None);
        };
        let run = self.finish_proposal_cutover(
            &proposal.id,
            &proposal.actor,
            &format!("revision-proposal:{}:drained", proposal.id),
        )?;
        let proposal = self
            .revision_proposal(&proposal.id)
            .map_err(internal)?
            .expect("the applied proposal exists");
        Ok(Some(RevisionSubmissionView {
            provenance: self
                .mission_provenance(&run.mission, &proposal.candidate_revision)
                .map_err(internal)?,
            status: "applied".into(),
            mission_run: run,
            proposal: Some(proposal),
        }))
    }

    fn unresolved_references(
            &self,
            intent: &NormalizedIntent,
        ) -> Result<Vec<String>, St3Error> {
        let connection = self.readers.get();
        crate::references::publication(
            intent,
            &ReferenceGraph {
                connection: &connection,
            },
            &self.origin,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    fn trim_checkpoint_for_test(&self, cut: u128) -> usize {
        let plan = plan_drops(&self.checkpoint_sealed_set(cut).unwrap());
        let mut writer = self.connection.write();
        let transaction = writer.transaction().unwrap();
        use smallclaims::store::checkpoint::{checkpoint_name, delete_dropped_rows_tx, record_checkpoint_tombstones_tx};
        record_checkpoint_tombstones_tx(&transaction, &checkpoint_name(cut), &plan.envelopes, &plan.claims)
            .unwrap();
        delete_dropped_rows_tx(&transaction, &plan.envelopes, &plan.claims).unwrap();
        transaction.commit().unwrap();
        plan.claims.len()
    }
}


#[cfg(any(test, feature = "test-support"))]
pub fn enrich_step_queue_for_reconcile_at(
    connection: &Connection,
    view: &mut StepRunView,
    snapshot_unix_ms: u128,
) -> rusqlite::Result<()> {
    enrich_step_queue_for_reconcile_cached(
        connection,
        view,
        snapshot_unix_ms,
        &mut PinnedDefinitions::default(),
    )
}

#[cfg(test)]
mod producer_guard {
    /// The part of st3-store's producer guard (`store::retirements`) for the message producers that moved up with this crate.
    #[test]
    fn moved_message_producers_use_the_common_wrapper() {
        for source in [
            include_str!("store/person_work.rs"),
            include_str!("store/adhoc_work.rs"),
            include_str!("store/github_workflow_failures.rs"),
        ] {
            assert!(source.contains("append_claim_tx(") && source.contains("\"message.sent\""));
        }
    }
}
