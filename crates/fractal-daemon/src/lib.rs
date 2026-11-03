//! st3 claims graph, API, reconciliation, and CLI support.

pub use fractal_kernel::cli_output;
// cli_output exports these macros at its crate root; this crate exported them at its own.
pub use fractal_kernel::{cli_print, cli_println};

#[cfg(any(test, feature = "test-support"))]
#[macro_use]
#[path = "../../../scripts/ci-test-paths.rs"]
#[allow(dead_code)]
mod ci_test_paths;

pub use fractal_kernel::accounts;
pub mod api;
pub mod web;
pub use fractal_kernel::archive;
pub mod attach_image;
pub use fractal_store::backup;
pub use fractal_store::blobs;
pub use fractal_store::owner_indexes;
pub use fractal_kernel::boot;
/// Git-backed change metadata and exact-OID atomic state transitions.
pub use fractal_github::changes;
pub use fractal_evidence::{EvidenceStore, OwnedSetsStore, PublicationFactsStore, PublicationStore, PublishedViewsStore};
pub use fractal_seats::{AccountsStore, EvidenceSeatsStore, HarnessQuestionsStore, LimitsStore, RetirementsStore, RevisionSeatsStore, RolloutsStore, RosterAdmissionStore, RuntimeStartsStore, SeatStatusStore, SeatsStore, SeatsStoreCore, SekretsDaemonStore, StopDeferralStore, SubagentsStore, TracePrincipalStore};
pub use fractal_missions::{AdhocWorkStore, ArrangementsStore, CoordinationStore, DeclarationsStore, DelegationStore, LanesStore, MissionEligibilityStore, MissionListStore, MissionsStore, MissionsStoreCore, PersonWorkStore, WorkListStore};
pub use fractal_delivery::{AgentMessagesStore, ConversationReadsStore, CustomStore, DeliveryStore, DeliveryStoreCore, DirectiveNotesStore, GlassesStore, MailboxChangesStore, MailboxLeaseStore, MailboxWakesStore, ProjectionDivergenceStore};
pub use fractal_attention::{AttentionListStore, AttentionSnapshotStore, AttentionStore, ClientSummaryStore, OwnerListsStore, PublishedListStore, PublishedViewsAttentionStore, SummaryListStore};
pub use fractal_github::{GitChangesStore, GithubStore, IdleHoldsStore, WatchesStore};
pub use fractal_api_mailbox::claude_channel;
pub use fractal_store::client;
pub use fractal_kernel::config;
pub use fractal_kernel::conversation_search;
pub use fractal_api_mailbox::creation;
pub use fractal_reconcile::repositories;
/// Answers the hooks an st3 seat's harness runs: `st driver-hook NAME`.
pub use fractal_api_mailbox::delivery_hold;
pub use fractal_reconcile::delivery_presence;
pub mod driver_hook;
pub use fractal_store::environment;
pub use fractal_kernel::evidence;
pub use fractal_api_mailbox::subagent_sessions;
/// Pairing grants a device the client gateway over Fabric; revoking it removes the grant.
pub use fractal_kernel::fabric_gateway;
pub use fractal_kernel::fleet;
/// Silent messages are stored and readable without waking the recipient.
pub use fractal_store::silent;
/// `st missions check`: run a mission file's exec gates once, now, the way a run would.
pub use fractal_api_mailbox::gate_check;
pub use fractal_kernel::gate_kinds;
pub use fractal_kernel::gate_report;
pub use fractal_store::github_watch;
pub use fractal_missions::graph;
pub use fractal_reconcile::handoff;
pub use fractal_api_mailbox::graph_watch_ivm;
pub use fractal_kernel::harness_events;
pub use fractal_kernel::hooks;
pub use fractal_telemetry::incremental;
pub use fractal_kernel::instance;
pub use fractal_kernel::lane;
pub use fractal_store::mailbox;
pub use fractal_store::message_body;
pub use fractal_delivery::maintenance;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub mod malloc_trim;
pub use fractal_kernel::read_cache;
pub use fractal_store::mission;
pub use fractal_kernel::model;
/// A driver relaunches its harness on the native session a suspended seat resumes.
pub use fractal_store::native_resume;
/// Durable seat-scoped first native launch outcomes.
pub use fractal_reconcile::native_seed;
pub use fractal_store::node_identity;
pub use fractal_api_mailbox::objective_faults;
pub use fractal_api_mailbox::objectives;
// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-02-omp-ask-resume-bridge — DELETE at contraction — https://app.notion.com/p/OMP-interrupted-ask-resume-bridge-st3-3ede3d41f4a3818a9e37ec160c006bbf
pub mod omp_ask_resume;
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-02-omp-ask-resume-bridge
pub use fractal_telemetry::otel;
pub use fractal_telemetry::otel_client_class;
pub use fractal_telemetry::otlp;
pub use fractal_peer::peer;
pub use fractal_reconcile::placement;
pub use fractal_store::person_request;
pub use fractal_kernel::pricing;
pub use fractal_store::private_notes;
pub use smallclaims::{performance, profile};
pub use fractal_kernel::projection;
pub use fractal_store::provenance;
pub use fractal_reconcile::reconcile;
pub use fractal_reconcile::schedule_standby;
pub use fractal_kernel::rules;
#[cfg(test)]
pub(crate) use fractal_kernel::publication;
pub use fractal_telemetry::recorder;
/// Imports local command receipts into durable resource observations.
pub use fractal_store::recorder_receipts;
pub use fractal_telemetry::recorder_report;
/// Finds references a publication or the graph names that do not resolve.
/// Attaches a local terminal to a terminal another fleet host owns.
pub use fractal_store::remote_terminal;
pub use fractal_store::render;
pub use fractal_store::resource;
pub use fractal_seats::rollout;
pub use fractal_kernel::seat_queue;
/// Runs any CLI with credentials no seat can read, through a gateway the sekrets user owns.
pub use fractal_store::sekrets;
pub use fractal_seats::sekrets_daemon;
pub mod service;
pub use fractal_kernel::skill;
pub use fractal_telemetry::slo;
pub use fractal_telemetry::slo_history;
pub use fractal_store::startup;
pub use fractal_telemetry::startup_telemetry;
pub use fractal_evidence::store;
pub mod subagents;
pub use fractal_seats::suspension;
pub use fractal_seats::external_sessions::omp_process_transcript;
pub use fractal_reconcile::seat_snapshot;
pub use fractal_telemetry::telemetry;
pub use smallclaims::task_cpu;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod kernel_tests;
#[cfg(test)]
mod peer_tests;
#[cfg(test)]
mod reconcile_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod telemetry_tests;
/// Attaches a local terminal to another fleet host's PTY session over Fabric, without st daemons.
pub mod terminal_fabric;
pub use fractal_kernel::terminal_binding;
pub use fractal_kernel::waiting_on;
/// Runs each agent as its own Unix user, in its own namespaces and root-owned cgroup.
pub use fractal_store::tile_registry;
pub mod tiles;
#[cfg(test)]
pub(crate) use fractal_telemetry::relay_trace;
#[cfg(test)]
pub(crate) use fractal_store::checkout;
#[cfg(test)]
pub(crate) use fractal_seats::external_sessions;

pub use graph::{parse_intent, validate_mission_runtimes};
pub use model::{NormalizedIntent, St3Error};
