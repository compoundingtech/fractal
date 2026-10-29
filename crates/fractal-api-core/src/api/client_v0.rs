


use crate::ConversationReadsStore;






use crate::MissionsStore;


use crate::AccountsStore;




use crate::SeatsStore;
use crate::SeatsStoreCore;


use super::*;


use axum::http::header::AUTHORIZATION;
use std::collections::BTreeSet;
use tracing::Instrument as _;

pub mod published_runtimes;

/// Socket ownership ends this root only after the first data frame reaches the client.
pub async fn first_frame_work<F: Future>(span: Option<tracing::Span>, work: F) -> F::Output {
    match span {
        Some(span) => work.instrument(span).await,
        None => work.await,
    }
}



pub mod raw_terminal;
pub mod resources;
pub mod search;
pub mod arrangements;
pub mod conversation_blocks;
pub mod conversation_delivery;
pub mod conversation_participants;
pub mod collection_windows;
pub mod inventory;
pub mod published_missions;
pub mod collection_ivm;
pub mod summary;
pub mod assistant;

pub const LOCAL_PERSON_HEADER: &str = "x-st3-person";

// A client holds one socket for all its current collection views. A subscription
// is a bounded window; history stays on the paged HTTP endpoints. A terminal is one
// more subscription on the same socket: whole screens, the latest only.
#[derive(Clone, Deserialize)]
pub struct CollectionSubscribe {
    pub kind: String,
    pub id: String,
    #[serde(default, deserialize_with = "collection_name")]
    pub collection: String,
    pub limit: Option<usize>,
    pub person: Option<String>,
    /// Follow one arrangement instead of the owner's bounded prefix window.
    pub subject: Option<String>,
    pub actor: Option<String>,
    /// Follow exactly one current agent instead of the fleet window.
    pub agent: Option<String>,
    pub status: Option<String>,
    /// Usage period boundaries; an omitted end follows new usage through the current time.
    since_ms: Option<u64>,
    until_ms: Option<u64>,
    /// Complete process coverage captured by an inventory read, before window truncation.
    #[serde(skip)]
    pub native_processes: Option<Arc<Mutex<Vec<u32>>>>,
    #[serde(skip)]
    pub inventory_deadline: Option<Arc<Mutex<Option<u128>>>>,
    /// A terminal subscription names the terminal, the incarnation `terminal.attach` fenced,
    /// and the single-use stream capability that attach returned.
    pub terminal: Option<String>,
    pub incarnation: Option<String>,
    pub capability: Option<String>,
    /// A conversation subscription names an agent or a session.
    pub conversation: Option<String>,
    /// Proposed frame context, pending #1910. Malformed telemetry never rejects a frame.
    // Duplicate top-level trace keys remain a serde error: clients must send one key.
    #[serde(rename = "trace", default, deserialize_with = "deserialize_frame_trace")]
    pub frame_trace: Option<crate::otel::FrameTraceContext>,
}
fn deserialize_frame_trace<'de, D>(
    deserializer: D,
) -> Result<Option<crate::otel::FrameTraceContext>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // deserialize_any would parse 1e999 before a Visitor can ignore it. RawValue uses
    // the ignored-value scanner instead, preserving the old unknown-field behavior.
    let raw = Box::<serde_json::value::RawValue>::deserialize(deserializer)?;
    if !raw.get().trim_start().starts_with('{') {
        return Ok(None);
    }
    struct TraceFields(crate::otel::FrameTraceContext);
    impl<'de> Deserialize<'de> for TraceFields {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct FieldsVisitor;
            impl<'de> serde::de::Visitor<'de> for FieldsVisitor {
                type Value = TraceFields;

                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("a trace context object")
                }

                fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                    let mut fields = crate::otel::FrameTraceContext::default();
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "traceparent" | "tracestate" => {
                                let raw = map.next_value::<&serde_json::value::RawValue>()?;
                                let value = serde_json::from_str::<String>(raw.get()).ok();
                                if key == "traceparent" {
                                    fields.traceparent = value;
                                } else {
                                    fields.tracestate = value;
                                }
                            }
                            _ => { map.next_value::<serde::de::IgnoredAny>()?; }
                        }
                    }
                    Ok(TraceFields(fields))
                }
            }
            deserializer.deserialize_map(FieldsVisitor)
        }
    }
    serde_json::from_str::<TraceFields>(raw.get())
        .map(|fields| Some(fields.0)).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod frame_trace_tests {
    use super::*;

    #[test]
    fn otel_frame_trace_preserves_ignored_json_decoding() {
        let deeply_nested = format!("{}0{}", "[".repeat(256), "]".repeat(256));
        for trace in [
            "1e999".to_owned(), deeply_nested.clone(), "null".to_owned(),
            r#"{"traceparent":1e999}"#.to_owned(),
            r#"{"traceparent":42,"tracestate":{}}"#.to_owned(),
            format!(r#"{{"traceparent":{deeply_nested},"tracestate":{deeply_nested},"unknown":{deeply_nested}}}"#),
        ] {
            let frame = format!(r#"{{"kind":"subscribe","id":"x","collection":"missions","trace":{trace}}}"#);
            let decoded: CollectionSubscribe = serde_json::from_str(&frame).unwrap();
            assert_eq!(decoded.kind, "subscribe");
            assert_eq!(decoded.collection, "missions");
            assert!(decoded.frame_trace.as_ref().is_none_or(|context|
                context.traceparent.is_none() && context.tracestate.is_none()), "{trace}");
        }
        let decoded: CollectionSubscribe = serde_json::from_str(
            r#"{"kind":"subscribe","id":"x","collection":"missions","trace":{"traceparent":"valid-string","tracestate":{}}}"#
        ).unwrap();
        let context = decoded.frame_trace.unwrap();
        assert_eq!(context.traceparent.as_deref(), Some("valid-string"));
        assert!(context.tracestate.is_none());
    }
}

/// A collection's name, with `alerts` read as `attention`: the same rows under the name a person
/// reads. Clients that predate alerts keep subscribing to `attention`.
fn collection_name<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let name = String::deserialize(deserializer)?;
    Ok(if name == "alerts" { "attention".into() } else { name })
}

pub struct CollectionSubscription {
    pub request: CollectionSubscribe,
    pub generation: u64,
    pub reading: Option<tokio::sync::oneshot::Sender<()>>,
    pub dirty: bool,
    /// Whether the client has this subscription's first snapshot.
    pub delivered: bool,
    pub previous: Arc<BTreeMap<String, Value>>,
    pub ivm: Option<Arc<collection_ivm::Adapter>>,
    pub cursor: Option<collection_ivm::Delivered>,
    pub order: Vec<String>,
    pub has_more: bool,
    /// Same-snapshot owner projection counter; only owner-wide arrangements use it.
    pub owner_revision: Option<u64>,
    /// The roster publication revision when this window's last read began. An agents window
    /// rereads once a later roster is published, even at the same graph index.
    pub roster_revision: u64,
    /// The same for every other published view: its revision when this window's last read began.
    pub view_revision: u64,
    /// When the client subscribed, until its first snapshot is sent.
    pub opened: Option<std::time::Instant>,
    /// The window's last read found its governed view not ready, and the client was told: it
    /// waits for the view's next change. A paired session's clock still reads it, so an expired
    /// or revoked grant still ends it, but another not-ready read sends nothing.
    pub waiting_for_view: bool,
}

// Observer grace periods and checkpoint waits can enter attention without a new claim.
pub const ATTENTION_CLOCK_INTERVAL: Duration = Duration::from_secs(30);
/// Claims that no collection window shows: rereading for them only costs.
pub fn collection_ignores(collection: &str, kind: &str) -> bool {
    // Summary overlays include transport reachability and agent activity as well as
    // graph membership. Its tiny public row is compared after each current read.
    if collection == "summary" {
        return false;
    }
    if collection == "summary-missions"
        && matches!(kind, "runtime.observed" | "harness.observed" | "harness.diagnostic"
            | "harness.activity" | "harness.usage" | "work.progress" | "work.submitted")
    {
        return true;
    }
    // A held window must be rechecked when its exact grant or delegated scopes change,
    // including otherwise graph-only glasses and arrangements.
    if matches!(kind, "custom.client.pairing-completed" | "custom.client.pairing-revoked") {
        return false;
    }
    match collection {
        "machines" if kind == "transport.observed" => return false,
        "usage" => return !kind.starts_with("document.") && !matches!(kind, "harness.usage" | "harness.limits" | "message.sent" | "message.staged" | "message.delivered"),
        "clients" | "devices" => return !kind.starts_with("custom.client.pairing-"),
        // Terminal declarations and imported session ownership
        // use the same graph inputs as the existing windows. Keep their conservative coverage.
        _ => {},
    }
    if collection == "glasses" { return !kind.starts_with("glass."); }
    if collection == "arrangements" {
        return !kind.starts_with("arrangement.")
            && !matches!(kind, "custom.client.pairing-completed" | "custom.client.pairing-revoked");
    }
    // Mission cards read run/step/gate state, definitions and person attention, never
    // harness telemetry. Attention additionally reads harness login state and diagnostics,
    // but not transcripts, token totals, todos or session-file metadata.
    if collection == "missions" && matches!(kind,
        "harness.observed" | "harness.diagnostic" | "harness.timeline" | "harness.usage"
            | "harness.todo.observed" | "harness.session-file") {
        return true;
    }
    if collection == "attention" && matches!(kind,
        "harness.timeline" | "harness.usage" | "harness.todo.observed" | "harness.session-file") {
        return true;
    }
    matches!(kind, "daemon.diagnostic" | "transport.observed" | "workspace.observed")
        || (kind == "harness.usage" && collection != "agents")
}

/// Advertised in `/v1/client/capabilities`: an agents subscription may select one exact agent.
/// Clients send `agent` only to a daemon that grants it; an older one ignores the selector.
pub const EXACT_AGENT_SELECTOR_CAPABILITY: &str = "collection.agents.exact-selector";
/// The longest exact agent selector; it is part of the shared window key.
pub const MAX_AGENT_SELECTOR_BYTES: usize = 256;

pub fn validate_agent_selector(collection: &str, agent: Option<&str>) -> Result<(), ApiError> {
    if let Some(agent) = agent {
        if collection != "agents" {
            return Err(validation("agent filters are supported for agents only"));
        }
        if !agent.starts_with("agent/") || agent.len() == "agent/".len()
            || agent.chars().any(char::is_whitespace)
        {
            return Err(validation("agent must be a full agent ID"));
        }
        if agent.len() > MAX_AGENT_SELECTOR_BYTES {
            return Err(validation(format!(
                "agent must be at most {MAX_AGENT_SELECTOR_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

pub const ALL_SCOPES: &[&str] = &[
    "read.projections",
    "write.client-diagnostics",
    "read.declarations",
    "read.glasses",
    "control.glasses",
    "read.arrangements",
    "control.arrangements",
    "terminal.read",
    "terminal.control",
    "control.attention",
    "control.messages",
    "control.launches",
    "control.missions",
    "control.work",
    "control.runtimes",
    "control.pairing",
];
const ACTIONS: &[&str] = &[
    "private-notes.write",
    "arrangement.edit",
    "harness.answer",
    "attention.resolve",
    "review.approve",
    "review.reject",
    "review.request-changes",
    "message.send",
    "message.read",
    "message.close",
    "launch.create",
    "launch.revise",
    "launch.preview",
    "launch.approve",
    "launch.cancel",
    "mission.start",
    "mission.revise",
    "mission.approve-revision",
    "mission.cancel-revision",
    "mission.cancel",
    "session.import",
    "work.ask",
    "custom.reply",
    "prompt.respond",
    "work.done",
    "work.cancel-ask",
    "work.claim",
    "work.renew",
    "work.progress",
    "work.complete",
    "work.fail",
    "work.release",
    "work.retry",
    "work.publish-mission",
    "agent.create",
    "agent.stop",
    "agent.retire",
    "agent.start",
    "agent.suspend",
    "agent.resume",
    "terminal.create",
    "terminal.end",
    "agent.queue-move",
    "lane.join",
    "lane.leave",
    "lane.move",
    "lane.mark",
    "lane.approve",
    "runtime.stop",
    "runtime.restart",
    "runtime.reset",
    "runtime.context-clear",
    "runtime.signal",
    "terminal.input",
    "terminal.resize",
    "terminal.attach",
    "terminal.detach",
    "pairing.revoke",
];
const AVAILABLE_ACTIONS: &[&str] = &[
    "private-notes.write",
    "custom.reply",
    "harness.answer",
    "prompt.respond",
    "arrangement.edit",
    "review.approve",
    "review.reject",
    "review.request-changes",
    "message.send",
    "message.read",
    "message.close",
    "launch.create",
    "launch.revise",
    "launch.preview",
    "launch.approve",
    "launch.cancel",
    "mission.start",
    "mission.approve-revision",
    "mission.cancel-revision",
    "mission.cancel",
    "session.import",
    "work.ask",
    "work.done",
    "work.cancel-ask",
    "work.claim",
    "work.renew",
    "work.progress",
    "work.complete",
    "work.fail",
    "work.release",
    "work.retry",
    "agent.create",
    "agent.stop",
    "agent.retire",
    "agent.start",
    "agent.suspend",
    "agent.resume",
    "terminal.create",
    "terminal.end",
    "agent.queue-move",
    "lane.join",
    "lane.leave",
    "lane.move",
    "lane.mark",
    "lane.approve",
    "runtime.stop",
    "runtime.restart",
    "runtime.reset",
    "runtime.context-clear",
    "runtime.signal",
    "terminal.input",
    "terminal.resize",
    "terminal.attach",
    "terminal.detach",
    "pairing.revoke",
];

/// Server-established browser admission, never deserialized from a request header.
/// This grants client API capabilities only; it is not an administrative bearer.
#[derive(Clone, Debug)]
pub struct LocalWebSession {
    pub person: String,
    pub id: String,
    pub expires_at: u128,
}

#[derive(Clone, Debug)]
pub struct ClientSession {
    /// The credential/session identity used for audit and idempotency isolation.
    pub actor: String,
    /// The concrete graph person or local agent whose authority is exercised.
    pub authority_actor: String,
    /// The exact pairing grant that authenticated this session. Two pairings of the same
    /// person and device key share an actor, so only this subject names the exact grant.
    pub pairing_grant: Option<String>,
    /// Local-only push registrations belong to the member that issued this exact pairing.
    pub pairing_issued_here: bool,
    pub grant_expires_at: Option<u128>,
    pub assistant_write: Option<crate::store::assistant_write::AssistantWriteAuthorization>,
    pub transport: &'static str,
    /// Server-established harness or local peer provenance, never a client header.
    pub local_peer: bool,
    /// Accept-time kernel connector generation; never supplied by a request header.
    pub peer_process: Option<fractal_runtime::PeerProcessIdentity>,
    pub custom_forms: bool,
    pub conversation_blocks: bool,
    pub conversation_delivery_links: bool,
    pub conversation_message_participants: bool,
    /// Internal assistant projection option, never negotiated by native clients.
    pub assistant_content_handles: bool,
    pub messages_paging: bool,
    pub scopes: std::collections::BTreeSet<String>,
}

impl ClientSession {
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(actor: &str, authority_actor: &str, transport: &'static str) -> Self {
        Self {
            actor: actor.into(),
            authority_actor: authority_actor.into(),
            pairing_grant: None,
            pairing_issued_here: false,
            grant_expires_at: None,
            assistant_write: None,
            transport,
            local_peer: false,
            peer_process: None,
            custom_forms: true,
            conversation_blocks: false,
            conversation_delivery_links: false,
            conversation_message_participants: false,
            assistant_content_handles: false,
            messages_paging: false,
            scopes: std::collections::BTreeSet::new(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests_scoped(
        actor: &str,
        authority_actor: &str,
        scopes: &[&str],
    ) -> Self {
        Self {
            scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
            ..Self::for_tests(actor, authority_actor, "unix")
        }
    }

    pub fn local(person: Option<&str>) -> Result<Self, ApiError> {
        let custom_forms = false;
        if person.is_some_and(|person| {
            !(person.starts_with("person/") && person.matches('/').count() == 1
                || person.starts_with("agent/"))
        }) {
            return Err(forbidden(
                "the trusted Unix client must identify one concrete person or agent",
            ));
        }
        let Some(person) = person else {
            return Ok(Self {
                actor: "client/local/read-only".into(),
                authority_actor: "client/local/read-only".into(),
                pairing_grant: None,
                pairing_issued_here: false,
            grant_expires_at: None,
                assistant_write: None,
                transport: "unix",
                local_peer: false,
                peer_process: None,
                custom_forms,
                conversation_blocks: false,
                conversation_delivery_links: false,
                conversation_message_participants: false,
                assistant_content_handles: false,
                messages_paging: false,
                scopes: ["read.projections", "terminal.read"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            });
        };
        // Free mode: an agent's local session holds every scope a person's does. Its actions
        // still record the agent as their actor.
        Ok(Self {
            actor: person.into(),
            authority_actor: person.into(),
            pairing_grant: None,
            pairing_issued_here: false,
            grant_expires_at: None,
            assistant_write: None,
            transport: "unix",
            local_peer: false,
            peer_process: None,
            custom_forms,
            conversation_blocks: false,
            conversation_delivery_links: false,
            conversation_message_participants: false,
            assistant_content_handles: false,
            messages_paging: false,
            scopes: ALL_SCOPES.iter().map(|scope| (*scope).to_owned()).collect(),
        })
    }

    pub fn pairing() -> Self {
        Self {
            actor: "client/pairing/completion".into(),
            authority_actor: "client/pairing/completion".into(),
            pairing_grant: None,
            pairing_issued_here: false,
            grant_expires_at: None,
            assistant_write: None,
            transport: "fabric-loopback",
            local_peer: false,
            peer_process: None,
            custom_forms: false,
            conversation_blocks: false,
            conversation_delivery_links: false,
            conversation_message_participants: false,
            assistant_content_handles: false,
            messages_paging: false,
            scopes: std::collections::BTreeSet::new(),
        }
    }

    pub fn allows(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }
}

/// Whether the session acts for a concrete person, or for a local agent seat. Within a fleet, an
/// agent may do whatever the person who runs the fleet may do (free mode).
pub fn acting_party(session: &ClientSession) -> bool {
    let actor = session.authority_actor.as_str();
    actor.starts_with("person/") && actor.matches('/').count() == 1
        || session.transport == "unix" && actor.starts_with("agent/")
}

pub fn capabilities(state: &AppState, session: &ClientSession) -> Vec<Value> {
    let mut capabilities = ALL_SCOPES
        .iter()
        .map(|scope| {
            json!({
                "id": scope,
                "version": if *scope == "write.client-diagnostics" { 1 } else { 0 },
                "state": if session.allows(scope) { "granted" } else { "ungranted" }
            })
        })
        .collect::<Vec<_>>();
    capabilities.push(json!({"id":"summary", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    // Attention rows say whether each is an alert and name its agent conversation; `alerts` names
    // the attention collection and routes, and the summary counts alerts.
    capabilities.push(json!({"id":"alerts", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"slo", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"inventory-collections", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"collections", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":EXACT_AGENT_SELECTOR_CAPABILITY, "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"custom-subjects", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    // `documents.list`, `document-references.get`, and `document.get` by bare name with its
    // version and changed time.
    capabilities.push(json!({"id":"documents", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"owned-sets", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"glasses", "version":2, "state":if glass_person(session, false).is_ok() && glass_person(session, true).is_ok() { "granted" } else { "ungranted" }}));
    capabilities.push(json!({"id":"arrangements", "version":1, "state":if session.allows("read.arrangements") && acting_party(session) { "granted" } else { "ungranted" }}));
    capabilities.extend(ACTIONS.iter().map(|action| {
        if *action == "harness.answer" {
            return super::harness_questions::capability(state, session);
        }
        let scope = action_scope(action).expect("registered client action has a scope");
        let state = if !AVAILABLE_ACTIONS.contains(action) {
            "unavailable"
        } else if session.allows(scope)
            && (!matches!(
                *action,
                "agent.create" | "agent.stop" | "agent.start" | "terminal.create" | "terminal.end"
            ) || require_creation_actor(session).is_ok())
        {
            "granted"
        } else {
            "ungranted"
        };
        json!({ "id": action, "version": 0, "state": state })
    }));
    capabilities.push(json!({"id":"device-key-proofs", "version":1, "state":"granted"}));
    // Pairing accepts `fabric_node_id` and reports `fabric_grant`.
    capabilities.push(json!({"id":"fabric-gateway-grant", "version":1, "state":"granted"}));
    capabilities.push(json!({"id":"conversation-message-participants", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"conversation-delivery-links", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"conversation-blocks", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"push-registration", "version":1, "state":if session.pairing_issued_here && session.transport != "unix" {"granted"} else {"unavailable"}}));
    capabilities.push(json!({"id":"messages-paging", "version":2, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities.push(json!({"id":"assistant.grants.v1", "version":1,
        "state":if assistant::trusted_issuer(state, session).is_ok() {"granted"} else {"ungranted"}}));
    // Agent rows carry st's presentation (docs/clients/ui-contract.md, Agent).
    capabilities.push(json!({"id":"presentation", "version":1, "state":if session.allows("read.projections") {"granted"} else {"ungranted"}}));
    capabilities
}

pub fn forbidden(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::FORBIDDEN,
        code: "forbidden".into(),
        message: message.into(),
        details: Box::default(),
    }
}

pub fn fabric_boundary_forbidden() -> ApiError {
    forbidden("the client gateway exposes only the authenticated client-v0 boundary")
}

pub fn validation(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: "validation-failed".into(),
        message: message.into(),
        details: Box::default(),
    }
}

pub fn stale(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::CONFLICT,
        code: "stale-fence".into(),
        message: message.into(),
        details: Box::default(),
    }
}

pub fn credential_digest(credential: &str) -> String {
    hex::encode(Sha256::digest(credential.as_bytes()))
}

/// Administrative reads must not inherit the general agent projection-read exemption.
pub fn administrative_get(request: &Request<Body>) -> bool {
    request.method() == axum::http::Method::GET
        && (request.uri().path().starts_with("/v1/client/assistant/grants/")
            || request.uri().path() == "/v1/client/devices")
}

pub fn authenticate(
    state: &AppState,
    request: &Request<Body>,
    transport: &'static str,
) -> Result<ClientSession, ApiError> {
    let custom_forms = request
        .headers()
        .get("x-st3-features")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|feature| feature.trim() == "custom-subjects.v1")
        });
    let conversation_blocks = request
        .headers()
        .get("x-st3-features")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|features| {
            features
                .split(',')
                .any(|feature| feature.trim() == "conversation-blocks.v1")
        });
    let conversation_message_participants = request.headers().get("x-st3-features")
        .and_then(|value| value.to_str().ok()).is_some_and(|value|
            value.split(',').any(|feature| feature.trim() == "conversation-message-participants.v1"));
    let conversation_delivery_links = request.headers().get("x-st3-features")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|feature|
            feature.trim() == "conversation-delivery-links.v1"));
    let messages_paging = request
        .headers()
        .get("x-st3-features")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|features| {
            features
                .split(',')
                .any(|feature| feature.trim() == "messages-paging.v2")
        });
    if let Some(web) = request.extensions().get::<LocalWebSession>() {
        let mut session = ClientSession::local(Some(&web.person))?;
        session.actor = format!("client/local-web/{}", web.id);
        session.transport = "local-web";
        session.custom_forms = custom_forms;
        session.grant_expires_at = Some(web.expires_at);
        session.conversation_blocks = conversation_blocks;
        session.messages_paging = messages_paging;
        return Ok(session);
    }
    let Some(value) = request.headers().get(AUTHORIZATION) else {
        if transport == "unix" {
            let person = request
                .headers()
                .get(LOCAL_PERSON_HEADER)
                .and_then(|value| value.to_str().ok());
            let mut session = ClientSession::local(person)?;
            session.peer_process = request.extensions().get::<super::LocalPeer>()
                .and_then(|peer| peer.peer_process.clone());
            session.local_peer = request.extensions().get::<super::BoundAgent>().is_some()
                || request.extensions().get::<super::LocalPeer>()
                    .is_some_and(|peer| peer.ancestor.is_some())
                || request.extensions().get::<super::NativeDeliveryPeer>().is_some();
            if session.local_peer && administrative_get(request)
                && session.authority_actor.starts_with("person/") {
                return Err(forbidden("local harnesses and peers cannot inspect owner administration"));
            }
            session.custom_forms = custom_forms;
            session.conversation_blocks = conversation_blocks;
            session.conversation_delivery_links = conversation_delivery_links;
            session.conversation_message_participants = conversation_message_participants;
            session.messages_paging = messages_paging;
            return Ok(session);
        }
        let pairing_completion = request.method() == axum::http::Method::POST
            && request.uri().path().starts_with("/v1/client/pairings/")
            && request.uri().path().ends_with("/complete");
        return pairing_completion
            .then(ClientSession::pairing)
            .ok_or_else(|| forbidden("the Fabric-loopback client credential is required"));
    };
    let value = value
        .to_str()
        .map_err(|_| forbidden("the client authorization header is malformed"))?;
    let credential = value
        .strip_prefix("Bearer ")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| forbidden("the client authorization scheme must be Bearer"))?;
    let digest = credential_digest(credential);
    if let Some(session) = assistant::authenticate(state, &digest, transport)? {
        assistant::authorize_route(&session, request)?;
        return Ok(session);
    }
    let pairings = state
        .store
        .claims_for_kind_at("custom.client.pairing-completed", None, true, 10_000)
        .map_err(ApiError::internal)?;
    let paired = pairings.claims.iter().find(|claim| {
        claim
            .body
            .pointer("/fields/credential_hash")
            .and_then(Value::as_str)
            == Some(digest.as_str())
    });
    let Some(paired) = paired else {
        return Err(forbidden("the client credential is unknown or expired"));
    };
    let mut session = paired_client_session(state, paired, transport, custom_forms)?;
    session.conversation_blocks = conversation_blocks;
    session.conversation_delivery_links = conversation_delivery_links;
    session.conversation_message_participants = conversation_message_participants;
    session.messages_paging = messages_paging;
    if request.method() == axum::http::Method::POST
        && request.uri().path() == "/v1/client/diagnostics"
    {
        require_scope(&session, "write.client-diagnostics")?;
    }
    if request.method() == axum::http::Method::GET {
        let scope = if request
            .uri()
            .path()
            .starts_with("/v1/client/agent-declarations/")
        {
            "read.declarations"
        } else if request.uri().path().starts_with("/v1/client/terminals/") {
            "terminal.read"
        } else {
            "read.projections"
        };
        require_scope(&session, scope)?;
    }
    Ok(session)
}

pub fn paired_client_session(
    state: &AppState,
    paired: &ClaimRecord,
    transport: &'static str,
    custom_forms: bool,
) -> Result<ClientSession, ApiError> {
    // Revoked wins: any revocation of the grant ends it, whatever order a member admitted the
    // pairing and the revocation in. Completing refuses a used grant, so nothing re-grants it.
    let revoked = !state.store.claims_for_subject_kind_at(
        &paired.subject, "custom.client.pairing-revoked", None, true, 1,
    ).map_err(ApiError::internal)?.claims.is_empty();
    let expires_at = paired.body.pointer("/fields/expires_at_unix_ms")
        .and_then(Value::as_u64).map(u128::from).unwrap_or_default();
    if revoked || expires_at <= client_now_ms() {
        return Err(forbidden("the client credential was revoked or expired"));
    }
    let actor = paired.body.pointer("/fields/session_actor").and_then(Value::as_str)
        .ok_or_else(|| ApiError::internal("a paired client has no derived actor"))?;
    let authority_actor = paired.body.pointer("/fields/person_id").and_then(Value::as_str)
        .filter(|actor| actor.starts_with("person/") && actor.matches('/').count() == 1)
        .ok_or_else(|| ApiError::internal("a paired client has no concrete delegated person"))?;
    let scopes = paired
        .body
        .pointer("/fields/scopes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    Ok(ClientSession {
        actor: actor.into(),
        authority_actor: authority_actor.into(),
        pairing_grant: Some(paired.subject.clone()),
        pairing_issued_here: paired.origin == state.store.origin(),
        grant_expires_at: Some(expires_at),
        assistant_write: None,
        transport,
        local_peer: false,
        peer_process: None,
        custom_forms,
        conversation_blocks: false,
        conversation_delivery_links: false,
        conversation_message_participants: false,
        assistant_content_handles: false,
        messages_paging: false,
        scopes,
    })
}

/// Held collection sockets carry no bearer secret. Re-resolve their session grant at each
/// arrangement read, using the same expiration/revocation checks as HTTP authentication.
pub fn revalidate_session(state: &AppState, session: &ClientSession) -> Result<ClientSession, ApiError> {
    if session.transport == "local-web" {
        // Only the server-established browser extension mints this transport.
        // Collection workers must retain its original expiry, never turn it into a pairing.
        if !acting_party(session)
            || state.owning_person.as_deref() != Some(session.authority_actor.as_str())
            || !session.actor.strip_prefix("client/local-web/")
                .is_some_and(|id| id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
            || !session.grant_expires_at.is_some_and(|expiry| expiry > client_now_ms())
            || session.pairing_grant.is_some()
        {
            return Err(forbidden("the local web session authority changed or expired"));
        }
        return Ok(session.clone());
    }
    if session.transport == "unix" && acting_party(session) {
        return Ok(session.clone());
    }
    // With a grant, the canonical newest completion, so every member checks the same one.
    let paired = match session.pairing_grant.as_deref() {
        Some(subject) => state.store
            .newest_claim_for_subject_kind(subject, "custom.client.pairing-completed")
            .map_err(ApiError::internal)?,
        None => state.store
            .claims_for_kind_at("custom.client.pairing-completed", None, true, 10_000)
            .map_err(ApiError::internal)?
            .claims.into_iter()
            .find(|claim| claim.body.pointer("/fields/session_actor").and_then(Value::as_str) == Some(session.actor.as_str())),
    }.ok_or_else(|| forbidden("the client session grant is absent"))?;
    let mut current = paired_client_session(state, &paired, session.transport, session.custom_forms)?;
    current.conversation_blocks = session.conversation_blocks;
    current.conversation_delivery_links = session.conversation_delivery_links;
    current.conversation_message_participants = session.conversation_message_participants;
    current.messages_paging = session.messages_paging;
    if current.authority_actor != session.authority_actor {
        return Err(forbidden("the client session authority changed"));
    }
    Ok(current)
}

pub fn require_scope(session: &ClientSession, scope: &str) -> Result<(), ApiError> {
    if session.allows(scope) {
        Ok(())
    } else {
        Err(forbidden(format!(
            "the authenticated client session is not granted `{scope}`"
        )))
    }
}

pub fn person_filter(
    session: &ClientSession,
    requested: Option<&str>,
) -> Result<Option<String>, ApiError> {
    if session.authority_actor.starts_with("person/") {
        if requested.is_some_and(|person| person != session.authority_actor) {
            return Err(forbidden(
                "the authenticated client cannot read another person's private projection",
            ));
        }
        return Ok(Some(session.authority_actor.clone()));
    }
    Ok(requested.map(str::to_owned))
}

/// [`mission_list_cards_at`] with the runs that wait on a person already read, as the published
/// missions list keeps them.
pub fn mission_list_cards_with(
    store: &Store,
    ids: &[String],
    at_unix_ms: u128,
    attention: &BTreeSet<String>,
) -> anyhow::Result<Vec<Value>> {
    let definitions = crate::performance::task("mission_collection/definitions", || store
        .mission_definitions_for_ids(ids))?
        .into_iter()
        .map(|d| (d.mission.subject.clone(), d))
        .collect::<BTreeMap<_, _>>();
    let resource_uris = store.declared_resource_uris(
        definitions
            .values()
            .flat_map(|definition| &definition.mission.resources),
    )?;
    ids.iter().map(|id| {
        let overview = crate::performance::task("mission_collection/overview", || store.mission_overview(id, 3))?;
        let newest = overview["newest"].as_array().expect("overview previews");
        let latest = newest.first();
        let definition = definitions.get(id);
        let state = if overview["counts"]["running"].as_u64().unwrap_or(0)>0 {"running"}
            else if overview["counts"]["standing"].as_u64().unwrap_or(0)>0 {"standing"}
            else if definition.is_some_and(|d| d.mission.state==crate::model::MissionState::Retired) {"retired"}
            else if let Some(run)=latest {run["status"].as_str().unwrap_or("ready")}
            else {match definition.map(|d| &d.mission.state) {
                Some(crate::model::MissionState::Draft)=>"draft",
                Some(crate::model::MissionState::Retired)=>"retired", _=>"ready"}};
        let updated = latest.and_then(|r| r["updated_at_unix_ms"].as_u64()).map(u128::from)
            .or_else(|| definition.map(|d| d.updated_at_unix_ms)).unwrap_or(0);
        let revision = latest.and_then(|r| r["revision"].as_str())
            .or_else(|| definition.map(|d| d.mission.revision.as_str())).unwrap_or("unknown");
        let active = overview["counts"].as_object().unwrap().iter()
            .filter(|(state,_)| !matches!(state.as_str(),"completed"|"failed"|"cancelled"))
            .map(|(_,count)| count.as_u64().unwrap_or(0)).sum::<u64>();
        let details = newest.iter().rev().map(|run| {
            let run_id=run["id"].as_str().expect("run header id");
            let (total,done,steps)=crate::performance::task("mission_collection/steps", || store.mission_step_preview_at(run_id, at_unix_ms))?;
            let terminal=matches!(run["status"].as_str(),Some("completed"|"failed"|"cancelled"));
            let scheduler_fault=store.reconcile_fault(run_id, crate::reconcile::FIRST_READINESS_FAULT_SCOPE)?;
            let must_act=if terminal {"nobody"} else if attention.contains(run_id) {"you"}
                else if scheduler_fault.is_some() {"blocked"}
                else if steps.iter().any(|s| matches!(s.status.as_str(),"ready"|"claimed"|"working")
                    && (s.assigned_to.is_some() || s.claimant.is_some() || !s.available_to.is_empty())) {"agent"}
                else if steps.iter().any(|s| s.status=="blocked") {"blocked"} else {"system"};
            let shown=steps.iter().map(|step| json!({
                "id":step.subject,"path":step.step,"title":step.title,"state":client_work_state(&step.status),
                "attempt":step.attempt,"assignee":step.assigned_to,"claimant":step.claimant,
                "agentless":step.agentless,"since":client_timestamp(step.updated_at_unix_ms),
                "keep_open":crate::mission::step_keeps_open(definition.map(|d| &d.mission), &step.step),
                "progress_at": null,
                "blocked_reason":step.blocked_reason.as_deref().or_else(|| scheduler_fault.as_deref().filter(|_| step.status=="pending")),"blockers":step.blockers,
                "goals":step.goals,"constraints":step.constraints
            })).collect::<Vec<_>>();
            let current=shown.iter().filter(|s| matches!(s["state"].as_str(),Some("ready"|"claimed"|"verifying"|"blocked")))
                .map(|s| json!({"id":s["id"],"title":s["title"],"assignee":s["assignee"],"claimant":s["claimant"],"state":s["state"],"since":s["since"],"progress_at":s["progress_at"]})).collect::<Vec<_>>();
            Ok::<Value,anyhow::Error>(json!({
                "id":run["id"],"generation_id":run["generation_id"],"requester":run["requester"],
                "status":run["status"],"phase":run["phase"],"progress":{"done":done,"total":total},
                "current_steps":current,"must_act":must_act,
                "blocker":scheduler_fault.map(|reason| json!({"step":steps.first().map(|step| &step.subject),"reason":reason,"scope":crate::reconcile::FIRST_READINESS_FAULT_SCOPE})),
                "state_since":client_timestamp(run["updated_at_unix_ms"].as_u64().unwrap_or(0) as u128),
                "steps":shown
            }))
        }).collect::<anyhow::Result<Vec<_>>>()?;
        let state=if details.iter().any(|run| run["blocker"]["scope"]==crate::reconcile::FIRST_READINESS_FAULT_SCOPE) {"blocked"} else {state};
        let must_act=["you","agent","blocked","system"].into_iter()
            .find(|kind| details.iter().any(|run| run["must_act"]==*kind)).unwrap_or(if active>0 {"system"} else {"nobody"});
        let generations=newest.iter().map(|r| (r["id"].as_str().unwrap().to_owned(),r["generation_id"].clone()))
            .collect::<serde_json::Map<_,_>>();
        let historical = matches!(state,"completed"|"failed"|"cancelled"|"retired");
        Ok(json!({"id":id,"kind":"mission","revision":revision,"updated_at":client_timestamp(updated),
            "title":id.trim_start_matches("mission/"),"state":state,"mission_revision":revision,
            "system":crate::mission::is_system(id, definition.map(|d| &d.mission)),
            "runs":newest.iter().rev().map(|r| r["id"].clone()).collect::<Vec<_>>(),
            "run_details":details,"active_runs":active,"total_runs":overview["total_runs"],
            "run_counts":overview["counts"],"runs_truncated":overview["total_runs"].as_u64().unwrap_or(0)>newest.len() as u64,
            "run_generations":generations,"must_act":must_act,
            "resources":crate::api::client_declared_resources(
                definition.map_or(&[][..], |d| d.mission.resources.as_slice()), &resource_uris),
            "operational":{"layer":if historical {"history"} else {"current"},"actionable":!historical,"reasons":[]}}))
    }).collect()
}

/// Assistant mission metadata keeps the native card's mission-level semantics,
/// but selects its relevant run independently of the bounded history preview.
/// Every step contributes scalar counts and agent IDs, even beyond step twenty.
pub fn assistant_mission_list_cards_with(
    store: &Store,
    ids: &[String],
    at_unix_ms: u128,
) -> anyhow::Result<Vec<Value>> {
    let mut cards = mission_list_cards_with(store, ids, at_unix_ms, &BTreeSet::new())?;
    for (id, card) in ids.iter().zip(&mut cards) {
        card["run_details"] = json!(
            store.assistant_mission_run_metadata_at(id, at_unix_ms)?
                .into_iter().collect::<Vec<_>>()
        );
    }
    Ok(cards)
}

pub fn bound_mission_cards(items: &mut Vec<Value>) -> anyhow::Result<bool> {
    // Reserve room for the envelope, continuation cursor and fleet sync notice.
    let budget = CLIENT_MAX_RESPONSE_BYTES.saturating_sub(128_000);
    let mut used = 0;
    let mut keep = 0;
    for item in items.iter_mut() {
        let mut bytes = serde_json::to_vec(item)?.len() + 1;
        if keep == 0 && bytes > budget {
            item["runs"] = json!([]);
            item["run_details"] = json!([]);
            item["runs_truncated"] = json!(true);
            bytes = serde_json::to_vec(item)?.len() + 1;
        }
        if used + bytes > budget {
            break;
        }
        used += bytes;
        keep += 1;
    }
    let truncated = keep < items.len();
    items.truncate(keep);
    anyhow::ensure!(
        !truncated || keep > 0,
        "a mission identifier exceeds the response budget"
    );
    Ok(truncated)
}

pub fn runtime_resources(
    state: &AppState,
    history: bool,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
) -> anyhow::Result<Vec<Value>> {
    runtime_resources_for_owner(state, history, snapshot, session, None)
}

pub fn runtime_resources_for_owner(
    state: &AppState,
    history: bool,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    owner: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    // Runtime authority must come from the same status reduction used by every
    // other control path. A raw claim ordered last by this replica's ingest
    // index is not necessarily the causally current runtime observation.
    // Exact owner reads reduce just that subject at this snapshot.
    let status = if let Some(owner) = owner {
        state.store.status_for_subject_names_at(
            BTreeSet::from([owner.to_owned()]),
            snapshot.store_index,
            history,
            crate::store::ClaimsReduction::Runtime,
        )?
    } else {
        state.store.status_for_claim_kind_at(
            "runtime.observed",
            Some(snapshot.store_index),
            history,
            crate::store::ClaimsReduction::Runtime,
        )?
    };
    runtime_resources_from_status(state, snapshot, session, status)
}

pub fn terminal_resources_for_owner(
    state: &AppState,
    history: bool,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    owner: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let status = state
        .store
        .terminal_resource_status_at(owner, snapshot.store_index, history)?;
    runtime_resources_from_status(state, snapshot, session, status)
}

pub fn runtime_resources_from_status(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    status: crate::model::StatusResponse,
) -> anyhow::Result<Vec<Value>> {
    runtime_resources_from_status_access(&state.store, &snapshot.id, &snapshot.created_at, status,
        session.allows("terminal.read"), session.allows("terminal.control"))
}

fn runtime_resources_from_status_access(
    store: &Store,
    _snapshot_id: &str,
    created_at: &str,
    status: crate::model::StatusResponse,
    read: bool,
    control: bool,
) -> anyhow::Result<Vec<Value>> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(count) = RUNTIME_DETAIL_SUBJECT_COUNTS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner).get_mut(_snapshot_id) {
        *count += status.subjects.len();
    }
    // Each runtime's declaration and observation time, in one statement apiece for the list.
    let desired_tokens = store.selected_desired_tokens(
        &status
            .subjects
            .iter()
            .map(|selected| selected.subject.as_str())
            .collect::<Vec<_>>(),
    )?;
    let claim_times = store.kept_claim_acceptance_times(
        &status
            .subjects
            .iter()
            .filter_map(|selected| selected.actual_claim.as_deref())
            .collect::<Vec<_>>(),
    )?;
    let mut values = Vec::new();
    for selected in status.subjects {
        let Some(actual) = selected.actual.as_ref() else {
            continue;
        };
        let fields = actual.get("fields").unwrap_or(actual);
        let Some(runtime_id) = fields.get("runtime_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(actual_claim) = selected.actual_claim.as_deref() else {
            continue;
        };
        let Some(actual_origin) = selected.actual_origin.as_deref() else {
            continue;
        };
        let observed = fields
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending");
        let mut runtime_state = match observed {
            "ready" | "working" | "idle" => "running",
            "absent" => "stopped",
            other @ ("pending" | "starting" | "running" | "stopping" | "stopped" | "exited"
            | "failed" | "unreachable") => other,
            _ => "pending",
        };
        let terminal = fields
            .get("terminal")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let owner_id = selected.subject.clone();
        let terminal_id = terminal.then(|| format!("terminal/{owner_id}"));
        let owner_host_id = client_host_id(actual_origin);
        let authoritative = matches!(selected.reachability.as_str(), "reachable" | "local");
        if !authoritative {
            runtime_state = "unreachable";
        }
        let local = authoritative && observed == "running" && actual_origin == store.origin();
        let incarnation_id = fields.get("incarnation_id").and_then(Value::as_str);
        let desired_revision = desired_tokens.get(&owner_id).cloned();
        let updated_at = claim_times
            .get(actual_claim)
            .map(|accepted_at| client_timestamp(*accepted_at))
            .unwrap_or_else(|| created_at.to_owned());
        let reasons = selected.reason.into_iter().collect::<Vec<_>>();
        values.push(json!({
            "id": format!("runtime/{runtime_id}"),
            "kind": "runtime",
            "revision": actual_claim,
            "updated_at": updated_at,
            "runtime_kind": if terminal { "terminal" } else { "agent" },
            "owner_id": owner_id,
            "owner_host_id": owner_host_id,
            "state": runtime_state,
            "runtime_id": runtime_id,
            "incarnation_id": incarnation_id,
            "desired_revision": desired_revision,
            "owner_run_id": selected.owner_run,
            "terminal_id": terminal_id,
            // Screen fences come from terminal.screen, never from a graph projection.
            "terminal_sequence": null,
            "terminal_access": terminal.then(|| json!({
                "read": if local && read { "granted" } else { "unavailable" },
                "input": if local && control { "granted" } else { "unavailable" },
                "resize": if local && control { "granted" } else { "unavailable" }
            })),
            "operational": {
                "layer": selected.projection.layer,
                "actionable": authoritative,
                "reasons": reasons,
                "runtime_incarnation": incarnation_id
            }
        }));
    }
    values.sort_by(|left, right| {
        left["owner_id"]
            .as_str()
            .cmp(&right["owner_id"].as_str())
            .then_with(|| {
                left["runtime_kind"]
                    .as_str()
                    .cmp(&right["runtime_kind"].as_str())
            })
            .then_with(|| left["id"].as_str().cmp(&right["id"].as_str()))
    });
    Ok(values)
}

pub fn machine_resources(
    state: &AppState,
    history: bool,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
) -> anyhow::Result<Vec<Value>> {
    machine_resources_with_work(state, history, snapshot, session, true)
}

pub fn machine_summary_resources(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
) -> anyhow::Result<Vec<Value>> {
    Ok(machine_summary_resources_and_hosts(state, snapshot, session)?.0)
}

/// The summary's machines and the host each current runtime runs on, by runtime subject.
pub fn machine_summary_resources_and_hosts(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
) -> anyhow::Result<(Vec<Value>, BTreeMap<String, String>)> {
    machine_resources_and_hosts(state, false, snapshot, session, false)
}

fn machine_resources_with_work(
    state: &AppState,
    history: bool,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    include_work: bool,
) -> anyhow::Result<Vec<Value>> {
    Ok(machine_resources_and_hosts(state, history, snapshot, session, include_work)?.0)
}

fn machine_resources_and_hosts(
    state: &AppState,
    history: bool,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    include_work: bool,
) -> anyhow::Result<(Vec<Value>, BTreeMap<String, String>)> {
    let runtimes = runtime_resources(state, history, snapshot, session)?;
    let mut host_runtime_ids = BTreeMap::<String, BTreeSet<String>>::new();
    let mut host_running_runtimes = BTreeMap::<String, usize>::new();
    let mut host_current_runtime_membership = BTreeSet::<String>::new();
    let mut runtime_owner_hosts = BTreeMap::<String, String>::new();
    let mut host_updated_at = BTreeMap::<String, String>::new();
    for runtime in &runtimes {
        let Some(host_id) = runtime["owner_host_id"].as_str() else {
            continue;
        };
        let Some(runtime_id) = runtime["id"].as_str() else {
            continue;
        };
        host_runtime_ids
            .entry(host_id.to_owned())
            .or_default()
            .insert(runtime_id.to_owned());
        if runtime["state"].as_str() == Some("running") {
            *host_running_runtimes.entry(host_id.to_owned()).or_default() += 1;
        }
        if runtime
            .pointer("/operational/layer")
            .and_then(Value::as_str)
            == Some("current")
        {
            host_current_runtime_membership.insert(host_id.to_owned());
        }
        if let Some(updated_at) = runtime["updated_at"].as_str() {
            host_updated_at
                .entry(host_id.to_owned())
                .and_modify(|current| {
                    if updated_at > current.as_str() {
                        *current = updated_at.to_owned();
                    }
                })
                .or_insert_with(|| updated_at.to_owned());
        }
        if let Some(owner_id) = runtime["owner_id"].as_str() {
            runtime_owner_hosts.insert(owner_id.to_owned(), host_id.to_owned());
        }
    }

    // Machines need only the claimant, subject, and update time of the steps a claimant holds.
    // Building full client work resources also reduces usage history, wake history and mission
    // annotations for every step, which makes this small host list expensive during the
    // startup burst when many seats connect at once.
    let work = if include_work {
        state.store.client_work_claims_at_snapshot(history, client_snapshot_time(snapshot))?
    } else {
        Vec::new()
    };
    let mut host_work = BTreeMap::<String, BTreeSet<String>>::new();
    for item in work {
        let Some(claimant) = item.claimant.as_deref() else {
            continue;
        };
        let Some(host_id) = runtime_owner_hosts.get(claimant) else {
            continue;
        };
        host_work
            .entry(host_id.clone())
            .or_default()
            .insert(item.subject);
        let updated_at = client_timestamp(item.updated_at_unix_ms);
        host_updated_at
            .entry(host_id.clone())
            .and_modify(|current| {
                if updated_at > *current {
                    *current = updated_at.clone();
                }
            })
            .or_insert(updated_at);
    }

    let local_host = client_host_id(&state.node);
    let mut host_ids = BTreeSet::from([local_host.clone()]);
    // Fleet members count as configured hosts; ended members are history. A config peer that
    // ended as a member is history too, even while its [[peers]] entry remains.
    let fleet = state.store.fleet_view_for_client()?;
    let ended_hosts = fleet
        .members
        .iter()
        .filter(|member| fleet.current(&member.name).is_empty())
        .map(|member| client_host_id(&member.name))
        .chain(fleet.legacy_removed.iter().map(|name| client_host_id(name)))
        .collect::<BTreeSet<_>>();
    let configured_hosts = state
        .configured_peers
        .iter()
        .map(|peer| client_host_id(peer))
        .chain(
            fleet
                .members
                .iter()
                .filter(|member| member.state != "ended")
                .map(|member| client_host_id(&member.name)),
        )
        .filter(|host| !ended_hosts.contains(host))
        .collect::<BTreeSet<_>>();
    host_ids.extend(configured_hosts.iter().cloned());
    host_ids.extend(host_runtime_ids.keys().cloned());
    let status = state.store.status_for_subject_prefix_at(
        "host/",
        Some(snapshot.store_index),
        history,
        crate::store::ClaimsReduction::Summary,
    )?;
    let mut host_statuses = status
        .subjects
        .into_iter()
        .filter(|subject| {
            subject.kind.as_deref() == Some("host") || subject.subject.starts_with("host/")
        })
        .map(|subject| (subject.subject.clone(), subject))
        .collect::<BTreeMap<_, _>>();
    if history {
        host_ids.extend(host_statuses.keys().cloned());
    }

    let mut machines = Vec::new();
    for host_id in host_ids {
        let name = host_id.strip_prefix("host/").unwrap_or(&host_id).to_owned();
        let current_member = host_id == local_host
            || configured_hosts.contains(&host_id)
            || host_current_runtime_membership.contains(&host_id);
        let mut updated_at = host_updated_at
            .get(&host_id)
            .cloned()
            .unwrap_or_else(|| client_timestamp(0));
        let (
            machine_state,
            transports,
            operational_layer,
            operational_actionable,
            operational_reasons,
        ) = if host_id != local_host && configured_hosts.contains(&host_id) {
            let (recent, last_success_at) = state.store.replication_peer_up(&name)?;
            (
                if recent { "reachable" } else { "last-seen" },
                vec![json!({
                    "protocol": "replication",
                    "status": if recent { "up" } else { "last-seen" },
                    "last_success_at": last_success_at.map(client_timestamp),
                })],
                "current".to_owned(),
                recent,
                vec!["replication-transport".to_owned()],
            )
        } else if host_id == local_host {
            (
                "local",
                vec![json!({
                    "protocol": "unix",
                    "status": "local",
                    "last_success_at": Value::Null,
                })],
                "current".to_owned(),
                true,
                vec!["authoritative-local-host".to_owned()],
            )
        } else if let Some(selected) = host_statuses.remove(&host_id) {
            let actual = selected.actual.as_ref();
            let fields = actual
                .and_then(|actual| actual.get("fields"))
                .or(actual)
                .unwrap_or(&Value::Null);
            let status = fields
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let conflicted = !selected.conflicts.is_empty()
                || !matches!(selected.reachability.as_str(), "reachable" | "local");
            let machine_state = match (conflicted, status) {
                (true, _) | (false, "unknown") => "indeterminate",
                (false, "up") => "reachable",
                (false, "down") => "unreachable",
                (false, _) => "indeterminate",
            };
            if let Some(claim) = selected.actual_claim.as_deref()
                && let Some(claim) = state.store.claim_by_id(claim)?
            {
                let claim_updated_at = client_timestamp(claim.accepted_at_unix_ms);
                if claim_updated_at > updated_at {
                    updated_at = claim_updated_at;
                }
            }
            let layer = if current_member {
                selected.projection.layer.clone()
            } else {
                "history".to_owned()
            };
            let mut reasons = selected.projection.reasons.clone();
            if current_member {
                reasons.push("replication-transport".to_owned());
            } else {
                reasons.push("discovered-history".to_owned());
            }
            if conflicted {
                reasons.push("authority-indeterminate".to_owned());
            }
            reasons.sort();
            reasons.dedup();
            // The transport claim changes only with the peer's status, so its success time
            // goes stale while the peer stays up. The peer row records every success.
            let last_success_at = fields
                .get("last_success_at")
                .and_then(Value::as_u64)
                .map(u128::from)
                .max(state.store.replication_peer_last_success(&name)?);
            (
                machine_state,
                vec![json!({
                    "protocol": fields.get("protocol").and_then(Value::as_str).unwrap_or("replication"),
                    "status": if matches!(status, "up" | "down") { status } else { "unknown" },
                    "last_success_at": last_success_at.map(client_timestamp),
                })],
                layer.clone(),
                layer == "current"
                    && selected.projection.actionable
                    && machine_state != "indeterminate",
                reasons,
            )
        } else {
            let reason = if configured_hosts.contains(&host_id) {
                "configured-unobserved"
            } else {
                "runtime-owner-host-unobserved"
            };
            (
                "indeterminate",
                vec![json!({
                    "protocol": "replication",
                    "status": "unknown",
                    "last_success_at": Value::Null,
                })],
                "current".to_owned(),
                false,
                vec![reason.to_owned()],
            )
        };
        let running_runtimes = host_running_runtimes.remove(&host_id).unwrap_or_default();
        let runtime_ids = host_runtime_ids
            .remove(&host_id)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        let work = host_work
            .remove(&host_id)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        let mut machine = json!({
            "host_id": host_id,
            "name": name.clone(),
            "state": machine_state,
            "fleet_id": state.fleet_id,
            "capacity": {
                "state": "unknown",
                "reason": "no capacity observation",
            },
            "occupancy": {
                "running_runtimes": running_runtimes,
            },
            "projects": [],
            "work": work,
            "transports": transports,
            "runtime_ids": runtime_ids,
            "operational": {
                "layer": operational_layer,
                "actionable": operational_actionable,
                "reasons": operational_reasons,
            }
        });
        let revision = format!(
            "machine:{}",
            hex::encode(Sha256::digest(
                serde_json::to_vec(&machine).expect("machine projection serializes")
            ))
        );
        let fields = machine
            .as_object_mut()
            .expect("machine projection is an object");
        fields.insert("id".into(), Value::String(format!("machine/{name}")));
        fields.insert("kind".into(), Value::String("machine".into()));
        fields.insert("revision".into(), Value::String(revision));
        fields.insert("updated_at".into(), Value::String(updated_at));
        machines.push(machine);
    }
    machines.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok((machines, runtime_owner_hosts))
}

pub struct OperationReport {
    pub store: std::sync::Weak<crate::store::Store>,
    /// `None` while the first report since the daemon started is being made.
    pub checks: Option<Arc<Vec<crate::model::DoctorCheck>>>,
    pub at: Instant,
    pub refreshing: bool,
}

/// The daemon's last diagnostic report for each store, which the operations collection lists.
/// Some checks compare the whole projection with the claim log, seconds of work on a busy host's
/// store, so page reads serve the last report and a new one is made off the request path.
static OPERATION_REPORTS: OnceLock<Mutex<BTreeMap<usize, OperationReport>>> = OnceLock::new();

pub fn operation_reports() -> std::sync::MutexGuard<'static, BTreeMap<usize, OperationReport>> {
    OPERATION_REPORTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Start the first diagnostic report off the request path and hand its operation audit to the
/// startup repair. Only this first report shares its audit; refreshes inspect current state.
pub fn start_operation_report(
    state: &AppState,
) -> Option<tokio::sync::oneshot::Receiver<anyhow::Result<Vec<String>>>> {
    let key = Arc::as_ptr(&state.store) as usize;
    let mut reports = operation_reports();
    if reports.get(&key).is_some_and(|report| {
        report
            .store
            .upgrade()
            .is_some_and(|store| Arc::ptr_eq(&store, &state.store))
    }) {
        return None;
    }
    reports.insert(
        key,
        OperationReport {
            store: Arc::downgrade(&state.store),
            checks: None,
            at: Instant::now(),
            refreshing: true,
        },
    );
    drop(reports);
    let (audit_sender, audit_receiver) = tokio::sync::oneshot::channel();
    let state = state.clone();
    std::thread::spawn(move || refresh_operation_report(&state, key, Some(audit_sender)));
    Some(audit_receiver)
}

pub fn refresh_operation_report(
    state: &AppState,
    key: usize,
    startup_audit: Option<tokio::sync::oneshot::Sender<anyhow::Result<Vec<String>>>>,
) {
    let report = if let Some(sender) = startup_audit {
        let drift = state.store.operation_projection_drift();
        let report = doctor_report_with_operation_drift(state, &drift);
        // The repair owns these findings, including any scan error. It re-derives just the
        // drifted rows from current claims, never reuses expected values from this snapshot.
        let _ = sender.send(drift);
        report
    } else {
        doctor_report(state)
    };
    let checks = report.ok().map(|report| Arc::new(report.0.checks));
    let mut reports = operation_reports();
    let Some(report) = reports.get_mut(&key) else {
        return;
    };
    report.refreshing = false;
    match checks {
        Some(checks) => {
            report.checks = Some(checks);
            report.at = Instant::now();
        }
        // The first report failed: the next read makes one and answers with its error.
        None if report.checks.is_none() => drop(reports.remove(&key)),
        None => {}
    }
}

/// Preserve the latest accepted observation even after its session or incarnation changes.
pub fn agent_todo_value(
    claim: Option<&ClaimRecord>,
    session: Option<&ClaimRecord>,
    incarnation: Option<&str>,
) -> Value {
    let Some(claim) = claim else {
        return Value::Null;
    };
    let snapshot = match fractal_schema::HarnessTodoSnapshot::deserialize(
        claim.body.get("fields").unwrap_or(&claim.body),
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(subject = %claim.subject, claim_id = %claim.id, %error,
                "invalid harness todo observation");
            return Value::Null;
        }
    };
    let session_fields = session.map(|claim| claim.body.get("fields").unwrap_or(&claim.body));
    let stale = incarnation != Some(snapshot.incarnation_id.as_str())
        || session_fields.and_then(|fields| fields["session_id"].as_str())
            != Some(snapshot.session_id.as_str());
    json!({
        "snapshot": snapshot,
        "claim_id": claim.id,
        "accepted_at": client_timestamp(claim.accepted_at_unix_ms),
        "stale": stale,
    })
}

pub async fn now(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Extension(session): Extension<ClientSession>,
    Query(query): Query<ClientListQuery>,
) -> Result<ClientPageResponse, ApiError> {
    require_scope(&session, "read.projections")?;
    let person = person_filter(&session, query.person.as_deref())?;
    let mut effective_query = query.clone();
    effective_query.person.clone_from(&person);
    let (history, actor, owner_run) = (query.history, query.actor.clone(), query.owner_run.clone());
    if history && actor.is_none() && owner_run.is_none() {
        return super::attention_history::page(state, snapshot, effective_query, "now").await;
    }
    client_snapshot_page(
        &state,
        snapshot,
        "now",
        &effective_query,
        move |state, snapshot| {
            let mut items =
                super::client_attention_resources_with_previews(state, person.as_deref(), history)?;
            client_attention_compatibility(&mut items, session.custom_forms);
            // The default Now view is the person's attention queue. Mission work belongs
            // in Control; only an explicit work filter opts it into this combined view.
            if actor.is_some() || owner_run.is_some() {
                let mut work = super::client_work_resources(
                    &state.store,
                    actor.as_deref(),
                    history,
                    client_snapshot_time(snapshot),
                    snapshot.store_index,
                )?;
                if let Some(owner_run) = owner_run.as_deref() {
                    work.retain(|item| item["mission_run_id"].as_str() == Some(owner_run));
                }
                items.extend(work);
            }
            // Attention is already ranked by the daemon. Keep that order when work is included.
            Ok(items)
        },
    )
    .await
}

pub fn device_resources(
    state: &AppState,
    snapshot: &ClientSnapshot,
    person: &str,
) -> Result<Vec<Value>, ApiError> {
    // Unrelated traffic must not hide recent devices. Read only indexed pairing history,
    // in bounded pages, at the requested snapshot; keep every page so old names and later
    // revocations remain visible even after many pairings.
    let mut claims = Vec::new();
    for kind in [
        "custom.client.pairing-begun",
        "custom.client.pairing-completed",
        "custom.client.pairing-revoked",
    ] {
        let mut before = snapshot.store_index.checked_add(1);
        loop {
            let page = state
                .store
                .claims_for_kind_at(kind, before, true, 256)
                .map_err(ApiError::internal)?;
            claims.extend(page.claims);
            let Some(cursor) = page.next_cursor else {
                break;
            };
            before = Some(cursor);
        }
    }
    claims.sort_by_key(|claim| claim.store_index);
    let mut paired = BTreeMap::<String, (&ClaimRecord, Option<&ClaimRecord>)>::new();
    let mut names = BTreeMap::<String, String>::new();
    for claim in &claims {
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        if claim.kind == "custom.client.pairing-begun"
            && let Some(name) = fields.get("device_name").and_then(Value::as_str)
        {
            names.insert(claim.subject.clone(), name.to_owned());
        }
        let Some(device_id) = fields.get("device_id").and_then(Value::as_str) else {
            continue;
        };
        match claim.kind.as_str() {
            "custom.client.pairing-completed"
                if fields.get("person_id").and_then(Value::as_str) == Some(person) =>
            {
                paired.insert(device_id.to_owned(), (claim, None));
            }
            "custom.client.pairing-revoked" => {
                if let Some(entry) = paired.get_mut(device_id) {
                    entry.1 = Some(claim);
                }
            }
            _ => {}
        }
    }
    // Pairing and revocation are fenced by the graph snapshot above. Expiration is
    // live state: its clock must advance even when no claim advances that snapshot.
    let at = client_now_ms();
    let push = state.store.push_registration_statuses(person, at as u64)
        .map_err(|_| ApiError::internal("local push registration status is unavailable"))?;
    let mut resources = paired
        .into_iter()
        .map(|(device_id, (completed, revoked))| {
            let fields = completed.body.get("fields").unwrap_or(&completed.body);
            let expires_at = fields
                .get("expires_at_unix_ms")
                .and_then(Value::as_u64)
                .map(u128::from)
                .unwrap_or_default();
            let selected = revoked.unwrap_or(completed);
            let state_name = if revoked.is_some() {
                "revoked"
            } else if expires_at <= at {
                "expired"
            } else {
                "active"
            };
            let mut resource = json!({
                "id": device_id,
                "kind": "device",
                "revision": selected.id,
                "updated_at": client_timestamp(selected.accepted_at_unix_ms),
                "person_id": person,
                "name": names.get(&completed.subject),
                "session_actor": fields.get("session_actor").cloned().unwrap_or(Value::Null),
                "state": state_name,
                "scopes": fields.get("scopes").cloned().unwrap_or_else(|| json!([])),
                "expires_at": client_timestamp(expires_at),
                "operational": {
                    "layer": if state_name == "active" { "current" } else { "history" },
                    "actionable": state_name == "active",
                    "reasons": if state_name == "active" { Vec::<&str>::new() } else { vec![state_name] }
                }
            });
            if completed.origin == state.node {
                resource["push"] = if state_name == "active" {
                    push.get(&device_id).map_or_else(
                        || json!({"registered":false}),
                        |status| json!(status),
                    )
                } else {
                    json!({"registered":false})
                };
            }
            resource
        })
        .collect::<Vec<_>>();
    resources.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok(resources)
}

pub fn client_connection_items(state: &AppState, snapshot: &ClientSnapshot) -> Result<Vec<Value>, ApiError> {
    let mut items = super::client_presence::list(&state.node, super::client_now_ms());
    // A paired phone acts as person/NAME/session/ID: name its device.
    let people = items
        .iter()
        .filter(|item| item["actor"].as_str().is_some_and(|actor| actor.contains("/session/")))
        .filter_map(|item| item["person"].as_str().map(str::to_owned))
        .filter(|person| person.starts_with("person/"))
        .collect::<BTreeSet<_>>();
    let mut devices = BTreeMap::<String, (Value, Value)>::new();
    for person in people {
        for device in device_resources(state, snapshot, &person)? {
            if let Some(actor) = device["session_actor"].as_str() {
                devices.insert(actor.to_owned(), (device["id"].clone(), device["name"].clone()));
            }
        }
    }
    for item in &mut items {
        if let Some((id, name)) = item["actor"].as_str().and_then(|actor| devices.get(actor)) {
            item["device_id"] = id.clone();
            item["device_name"] = name.clone();
        }
    }
    Ok(items)
}

pub fn timeline_attribution(owner: &str, desired: &[crate::model::DesiredSubject]) -> Value {
    let ownership = desired.iter().find(|desired| desired.subject == owner);
    json!({
        "agent_id": owner,
        "mission_run_id": ownership.and_then(|value| value.owner_run.as_deref()),
        "generation_id": ownership.and_then(|value| value.owner_generation.as_deref()),
        "step_id": ownership.and_then(|value| value.owner_step.as_deref()),
    })
}


fn normalized_timeline_usage_body(
    body: Value,
    attribution: &Value,
    source_driver: Option<&str>,
) -> Value {
    let mut body = body.as_object().cloned().unwrap_or_default();
    if !matches!(
        body.get("semantics").and_then(Value::as_str),
        Some("context_occupancy" | "session_cumulative" | "response")
    ) {
        // Legacy/provider events without declared cumulative semantics are one
        // response observation; treating them as cumulative would undercount.
        body.insert("semantics".into(), Value::String("response".into()));
    }
    if let Some(driver) = source_driver.filter(|driver| !driver.is_empty()) {
        body.insert("driver".into(), Value::String(driver.into()));
    } else if !body
        .get("driver")
        .and_then(Value::as_str)
        .is_some_and(|driver| !driver.is_empty())
    {
        body.insert("driver".into(), Value::String("unknown".into()));
    }
    body.insert("attribution".into(), attribution.clone());
    Value::Object(body)
}

/// The Fractal in one session's conversation: messages sent for this session, and messages
/// to or from its agent that name no session, accepted while this incarnation was the agent's
/// current one. st joins them into the timeline so every client shows the same conversation.
fn session_messages(
    state: &AppState,
    owner: &str,
    session_id: &str,
    incarnation: Option<&str>,
    before: Option<u64>,
) -> Result<Vec<ClaimRecord>, ApiError> {
    let (started, ended) = match incarnation {
        Some(incarnation) => state.store.conversation_runtime_span(owner, incarnation)
            .map_err(ApiError::internal)?,
        None => (None, None),
    };
    let mut messages = state.store.conversation_messages_at(owner, before, 0)
        .map_err(ApiError::internal)?;
    messages.retain(|claim| session_message_belongs(claim, owner, session_id, started, ended));
    Ok(messages)
}

fn session_message_belongs(
    claim: &ClaimRecord,
    owner: &str,
    session_id: &str,
    started: Option<u128>,
    ended: Option<u128>,
) -> bool {
    let fields = claim.body.get("fields").unwrap_or(&claim.body);
    if fields["from"].as_str() != Some(owner) && fields["to"].as_str() != Some(owner) {
        return false;
    }
    match fields["session_id"].as_str() {
        Some(message_session) => message_session == session_id,
        None => started.is_none_or(|started| claim.accepted_at_unix_ms >= started)
            && ended.is_none_or(|ended| claim.accepted_at_unix_ms < ended),
    }
}

/// A message's timeline body: who wrote to whom, about what, so a client can draw Fractal
/// apart from the harness's own turns.
/// Who signed a message a person wrote and whether it checks: the verdict st recorded, and the
/// device the signing key was granted to, by the label the person gave it when it paired. A
/// message an agent wrote carries none: its key is the node's, and only people's words need
/// their provenance shown.
pub fn message_provenance(state: &AppState, claim: &ClaimRecord) -> Option<Value> {
    let fields = claim.body.get("fields").unwrap_or(&claim.body);
    if !fields
        .get("from")
        .and_then(Value::as_str)?
        .starts_with("person/")
    {
        return None;
    }
    let authorization = state.store.claim_authorization(&claim.id).ok()?;
    let verdict = authorization.verdict;
    let mut provenance = json!({"verdict": verdict.name(), "authorization_model": authorization.model});
    if let smallclaims::principal::Verdict::Held(reason)
    | smallclaims::principal::Verdict::Invalid(reason) = &verdict
    {
        provenance["reason"] = json!(reason);
    }
    if let Some(signature) = authorization.signature {
        provenance["signer"] = json!(signature.signer);
        provenance["key"] = json!(signature.key);
        let label = signature
            .chain
            .first()
            .and_then(|grant| state.store.claim_by_id(grant).ok().flatten())
            .and_then(|grant| key_grant_label(&grant.body));
        if let Some(label) = label {
            provenance["device"] = json!(label);
        }
    }
    Some(provenance)
}

/// The label a device gave its key when it paired, from the grant claim's body.
pub fn key_grant_label(body: &Value) -> Option<String> {
    body.pointer("/fields/label")
        .and_then(Value::as_str)
        .filter(|label| !label.trim().is_empty())
        .map(str::to_owned)
}

pub fn session_message_content(session: &ClientSession, session_id: &str, claim: &ClaimRecord) -> Value {
    let fields = claim.body.get("fields").unwrap_or(&claim.body);
    let mut body = json!({"media_type":"text/plain", "text":fields.get("content").and_then(Value::as_str).unwrap_or_default()});
    if session.assistant_content_handles {
        body["blocks"] = json!([{"kind":"raw_text", "source_type":"content", "payload":{"body_ref":true},
            "continuation":{"ref":conversation_blocks::message_reference(session_id, &claim.id)}}]);
    }
    body
}

pub fn session_message_body(state: &AppState, claim: &ClaimRecord, participants: bool) -> Value {
    let fields = claim.body.get("fields").unwrap_or(&claim.body);
    let mut body = json!({
        "message_id": claim.subject,
        "reply_to": fields.get("in_reply_to"),
    });
    conversation_participants::apply(&mut body, claim, participants);
    if let Some(title) = fields.get("title").and_then(Value::as_str) {
        body["title"] = Value::String(title.to_owned());
    }
    if let Some(provenance) = message_provenance(state, claim) {
        body["provenance"] = provenance;
    }
    if let Some(tags) = fields.get("tags").and_then(Value::as_array)
        && !tags.is_empty()
    {
        body["tags"] = Value::Array(tags.clone());
    }
    let attachments = crate::message_body::split_attachments(fields.get("attachments")).0;
    if !attachments.is_empty() {
        body["attachments"] = attachments.iter().map(super::client_attachment).collect();
    }
    if let Some(long) = crate::message_body::split_attachments(fields.get("attachments")).1 {
        body["body_bytes"] = long.size.into();
    }
    if let Some(class) = fields.get("sender_class").and_then(Value::as_str) {
        body["sender_class"] = Value::String(class.to_owned());
    }
    if let Some(grant) = fields.get("assistant_grant_id").and_then(Value::as_str) {
        body["assistant_grant_id"] = Value::String(grant.to_owned());
    }
    body
}

fn timeline_order(a: &Value, b: &Value) -> std::cmp::Ordering {
    crate::external_sessions::compare_timeline_positions(
        (a["timestamp"].as_str().unwrap_or_default(), a["sequence"].as_u64().unwrap_or(0), 0, a["id"].as_str().unwrap_or_default()),
        (b["timestamp"].as_str().unwrap_or_default(), b["sequence"].as_u64().unwrap_or(0), 0, b["id"].as_str().unwrap_or_default()),
        crate::external_sessions::TimelineOrder::TimestampSequence,
    )
}

/// The conversation header (client contract `conversation-blocks.v1` §3): derived from a
/// normalized bounded window, with live register values winning once the register (#1583)
/// exists. Native keyset pages pass their newest bounded head, not an offset page cache.
fn conversation_page_header(
    state: &AppState,
    session: &ClientSession,
    session_id: &str,
    items: &[Value],
    window_truncated: bool,
) -> Option<Value> {
    if !session.conversation_blocks {
        return None;
    }
    let _header = crate::profile::span("conversation/header");
    let derived = crate::conversation_header::derive(
        items,
        window_truncated || crate::conversation_header::window_truncated(items),
    );
    crate::conversation_header::merge_register(
        derived,
        conversation_register_value(state, session_id).as_ref(),
    )
}

/// The live latest-wins values for a conversation, e.g. `working`. Wire the register here when
/// draft #1583 (the live conversation register) lands; until then no register exists and the
/// header is derived from the transcript window alone.
fn conversation_register_value(_state: &AppState, _session_id: &str) -> Option<Value> {
    None
}

/// Only offset continuations (OpenCode and claim timelines) retain their original window
/// in the page cache. Native keyset continuations derive a bounded head in native_slice_page.
fn offset_continuation_header(
    state: &AppState,
    session: &ClientSession,
    session_id: &str,
    snapshot: &ClientSnapshot,
    cursor: &str,
) -> Option<Value> {
    if !session.conversation_blocks {
        return None;
    }
    let cursor = super::decode_client_cursor(cursor).ok()?;
    let collection = format!("timeline/{session_id}");
    if cursor.collection != collection || cursor.snapshot.id != snapshot.id {
        return None;
    }
    let items = {
        let cache = super::client_page_cache()
            .lock()
            .expect("client page cache poisoned");
        cache
            .iter()
            .find(|entry| {
                entry.snapshot_id == cursor.snapshot.id
                    && entry.collection == collection
                    && entry.items_digest == cursor.items_digest
            })?
            .items
            .clone()
    };
    conversation_page_header(state, session, session_id, &items, false)
}

pub fn native_session_messages(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    session_id: &str,
    managed: Option<&super::ManagedSessionOwner>,
) -> Result<Vec<Value>, ApiError> {
    let mut items = Vec::new();
    if let Some((owner, incarnation, _)) = managed {
        let messages = session_messages(
            state,
            owner,
            session_id,
            incarnation.as_deref(),
            snapshot.store_index.checked_add(1),
        )?;
        for claim in messages {
            let fields = claim.body.get("fields").unwrap_or(&claim.body);
            let from = fields.get("from").and_then(Value::as_str);
            let role = if from == Some(owner.as_str()) {
                "assistant"
            } else {
                "user"
            };
            let stamp = client_timestamp(claim.accepted_at_unix_ms);
            let digest = hex::encode(Sha256::digest(claim.id.as_bytes()));
            let base = claim.store_index.saturating_mul(4);
            items.push(json!({"id":format!("timeline-entry/{}/{}-message", session_id.trim_start_matches("session/"), &digest[..16]), "sequence":base, "revision":1, "timestamp":stamp, "role":role, "type":"message", "final":true, "body":session_message_body(state, &claim, session.conversation_message_participants)}));
            items.push(json!({"id":format!("timeline-entry/{}/{}-content", session_id.trim_start_matches("session/"), &digest[..16]), "sequence":base+1, "revision":1, "timestamp":stamp, "role":role, "type":"content", "final":true, "body":session_message_content(session, session_id, &claim)}));
        }
        items.sort_by(timeline_order);
    }
    items.reverse();
    Ok(items)
}

/// Remove only exact delivery echoes; graph rows remain the canonical projection.
fn suppress_native_echoes(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session_id: &str,
    graph: &[Value],
    items: &mut Vec<Value>,
    managed: Option<&super::ManagedSessionOwner>,
) -> Result<u64, ApiError> {
    let mut suppressed = 0;
    if let Some((owner, incarnation, _)) = managed {
        let mut incoming = graph.iter().filter_map(|item| {
            let body = &item["body"];
            (body["to"].as_str() == Some(owner.as_str())
                && body["from"].as_str() != Some(owner.as_str()))
                .then(|| body["message_id"].as_str()).flatten()
        }).collect::<BTreeSet<_>>();
        let future = post_snapshot_native_sends(
            state, snapshot, owner, session_id, incarnation.as_deref(), items, &incoming,
        )?;
        incoming.extend(future.iter().filter_map(|claim| {
            let fields = claim.body.get("fields").unwrap_or(&claim.body);
            (fields["to"].as_str() == Some(owner.as_str())
                && fields["from"].as_str() != Some(owner.as_str()))
                .then_some(claim.subject.as_str())
        }));
        let codex_ids = items.iter()
            .filter_map(|item| item.pointer("/_delivery_correlation/thread_id").and_then(Value::as_str))
            .collect::<BTreeSet<_>>().into_iter().map(|thread| {
                (thread.to_owned(), incoming.iter().map(|subject| {
                    fractal_drivers::codex_app_server::stable_client_user_message_id(owner, thread, subject)
                }).collect::<BTreeSet<_>>())
            }).collect::<BTreeMap<_, _>>();
        items.retain(|item| {
            let echo = item.get("_delivery_correlation")
                .is_some_and(|correlation| native_echo_matches(correlation, &incoming, &codex_ids));
            if echo {
                suppressed = suppressed.max(item["sequence"].as_u64().unwrap_or(0));
            }
            !echo
        });
    }
    for item in items {
        item.as_object_mut().expect("native timeline entry is an object")
            .remove("_delivery_correlation");
    }
    Ok(suppressed)
}

pub fn native_timeline_page(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    session_id: &str,
    query: &ClientListQuery,
    mut items: Vec<Value>,
    managed: Option<&super::ManagedSessionOwner>,
) -> Result<Json<Value>, ApiError> {
    let order = native_timeline_order(managed);
    let graph = if order == crate::external_sessions::TimelineOrder::TimestampSequence {
        native_session_messages(state, snapshot, session, session_id, managed)?
    } else {
        Vec::new()
    };
    suppress_native_echoes(state, snapshot, session_id, &graph, &mut items, managed)?;
    let mut keyed: Vec<_> = items.drain(..)
        .map(|item| {
            let key = crate::external_sessions::timeline_key(&item, 0);
            (item, key)
        })
        .collect();
    if order == crate::external_sessions::TimelineOrder::TimestampSequence {
        keyed.extend(graph.into_iter()
            .map(|item| {
                let key = crate::external_sessions::timeline_key(&item, 1);
                (item, key)
            }));
    }
    keyed.sort_by(|(_, a), (_, b)| a.cmp_in(b, order));
    items = keyed.into_iter().rev().map(|(item, _)| item).collect();
    let header = conversation_page_header(state, session, session_id, &items, false);
    let mut page = client_page(
        state,
        snapshot,
        &format!("timeline/{session_id}"),
        items,
        query,
    )?;
    page.items.reverse();
    let mut page_value = json!({
        "kind": "timeline-page",
        "session_id": session_id,
        "items": page.items,
        "page": page.page
    });
    if let Some(header) = header {
        page_value["header"] = header;
    }
    Ok(Json(page_value))
}

pub fn native_timeline_order(
    managed: Option<&super::ManagedSessionOwner>,
) -> crate::external_sessions::TimelineOrder {
    if managed.is_some() {
        crate::external_sessions::TimelineOrder::TimestampSequence
    } else {
        crate::external_sessions::TimelineOrder::Sequence
    }
}

pub const NATIVE_PAGE_CURSOR_PREFIX: &str = "page/native/";

/// All stateless cursor authority is authenticated, including expiry, the source basis, and
/// the source's content generation. A process restart changes the key, invalidating these
/// pages just like cached offset pages.
#[derive(Serialize, Deserialize)]
pub struct NativePagePosition {
    session_id: String,
    collection: String,
    order: String,
    boundary: crate::external_sessions::TimelineKey,
    limit: usize,
    filter_digest: String,
    snapshot: ClientSnapshot,
    source_basis: String,
    /// Content generation at the page boundary. An in-place replacement moves it even when
    /// driver, native id, path and inode are unchanged.
    source_generation: u64,
    expires_at_unix_ms: u128,
}

#[derive(Serialize, Deserialize)]
struct NativePageCursor {
    position: NativePagePosition,
    #[serde(default)]
    mac: Option<String>,
}

fn native_page_mac(position: &NativePagePosition) -> Result<hmac::Hmac<Sha256>, ApiError> {
    use hmac::Mac as _;
    static KEY: std::sync::LazyLock<[u8; 32]> = std::sync::LazyLock::new(|| {
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key).expect("creating the native page cursor authentication key");
        key
    });
    let mut mac = <hmac::Hmac<Sha256> as hmac::Mac>::new_from_slice(&*KEY)
        .expect("HMAC accepts a 32-byte key");
    // Struct serialization fixes the field order; the position contains no arbitrary JSON.
    mac.update(&serde_json::to_vec(position).map_err(ApiError::internal)?);
    Ok(mac)
}

fn encode_native_page_cursor(position: NativePagePosition) -> Result<String, ApiError> {
    use hmac::Mac as _;
    let mac = hex::encode(native_page_mac(&position)?.finalize().into_bytes());
    let encoded = serde_json::to_vec(&NativePageCursor { position, mac: Some(mac) })
        .map_err(ApiError::internal)?;
    Ok(format!("{NATIVE_PAGE_CURSOR_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(encoded)))
}

fn decode_native_page_cursor(encoded: &str) -> Result<NativePagePosition, ApiError> {
    use hmac::Mac as _;
    let expired = || client_page_expired("the page snapshot is no longer available; restart pagination");
    let encoded = encoded.strip_prefix(NATIVE_PAGE_CURSOR_PREFIX).ok_or_else(expired)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded)
        .map_err(|_| expired())?;
    let cursor: NativePageCursor = serde_json::from_slice(&bytes).map_err(|_| expired())?;
    let signature = hex::decode(cursor.mac.as_deref().ok_or_else(expired)?)
        .map_err(|_| expired())?;
    native_page_mac(&cursor.position)?.verify_slice(&signature).map_err(|_| expired())?;
    Ok(cursor.position)
}

pub fn native_page_cursor_snapshot(encoded: &str) -> Result<ClientSnapshot, ApiError> {
    decode_native_page_cursor(encoded).map(|position| position.snapshot)
}

fn native_page_filter_digest(query: &ClientListQuery) -> Result<String, ApiError> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&json!([
        query.history, query.person, query.actor, query.owner_run, query.status,
        query.native_only, query.owner, query.state
    ])).map_err(ApiError::internal)?)))
}

/// Native pages retain no whole-window page cache. Their authenticated boundary names
/// the entire last-returned key, including source rank, so tied entries survive pagination.
pub fn native_slice_page(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    session_id: &str,
    query: &ClientListQuery,
    source: &crate::external_sessions::ExternalSession,
) -> Result<Json<Value>, ApiError> {
    let managed = super::managed_session_owner_at(&state.store, snapshot.store_index, session_id)
        .map_err(ApiError::internal)?;
    native_slice_page_at_generation(state, snapshot, session, session_id, query, source, managed.as_ref())
        .map(|(page, _, _)| page)
}

/// [`native_slice_page`] together with the source content generation it was read at.
pub fn native_slice_page_at_generation(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    session_id: &str,
    query: &ClientListQuery,
    source: &crate::external_sessions::ExternalSession,
    managed: Option<&super::ManagedSessionOwner>,
) -> Result<(Json<Value>, u64, u64), ApiError> {
    let preparation = crate::profile::span("conversation/native-preparation");
    let cursor = query.cursor.as_deref().map(decode_native_page_cursor).transpose()?;
    let source_basis = conversation_blocks::basis(source)?;
    let order = native_timeline_order(managed);
    let order_name = match order {
        crate::external_sessions::TimelineOrder::Sequence => "sequence",
        crate::external_sessions::TimelineOrder::TimestampSequence => "timestamp-sequence",
    };
    let filter_digest = native_page_filter_digest(query)?;
    let requested_limit = query.limit.unwrap_or(CLIENT_DEFAULT_PAGE_ITEMS)
        .clamp(1, CLIENT_MAX_PAGE_ITEMS);
    if let Some(cursor) = &cursor {
        if cursor.session_id != session_id
            || cursor.collection != format!("timeline/{session_id}")
            || cursor.snapshot.id != snapshot.id
            || cursor.snapshot.store_index != snapshot.store_index
            || cursor.order != order_name
            || cursor.filter_digest != filter_digest
            || cursor.source_basis != source_basis
            || query.limit.is_some_and(|_| requested_limit != cursor.limit)
        {
            return Err(client_page_expired(
                "the page cursor does not match this collection, snapshot, or filter",
            ));
        }
        if client_now_ms() > cursor.expires_at_unix_ms {
            return Err(client_page_expired("the page cursor expired"));
        }
    }
    let limit = cursor.as_ref().map_or(requested_limit, |cursor| cursor.limit);
    let expires_at_unix_ms = cursor.as_ref().map_or_else(
        || client_now_ms().saturating_add(CLIENT_PAGE_TTL_MS),
        |cursor| cursor.expires_at_unix_ms,
    );
    let before = cursor.as_ref().map(|cursor| &cursor.boundary);
    // A keyset header summarizes the newest 200 native items, not the older page being
    // served. Reuse the first-page read for that head; continuations read a bounded head
    // separately. Derive once per request, never scan the whole window or use the offset cache.
    let read_limit = if session.conversation_blocks && before.is_none() {
        (limit + 1).max(CLIENT_MAX_PAGE_ITEMS)
    } else {
        limit + 1
    };
    let messages = if order == crate::external_sessions::TimelineOrder::TimestampSequence {
        native_session_messages(state, snapshot, session, session_id, managed)?
    } else {
        Vec::new()
    };
    drop(preparation);
    let (mut native, suppressed_sequence) = canonical_native_slice(
        state, snapshot, source, session, session_id, &messages, order, before, read_limit, managed, true,
    )?;
    if native.basis != source_basis {
        return Err(client_page_expired("the transcript changed while reading the page"));
    }
    if let Some(cursor) = &cursor
        && cursor.source_generation != native.generation
    {
        return Err(client_page_expired(
            "the transcript was replaced while paging; restart pagination",
        ));
    }
    let header = if session.conversation_blocks {
        let head = if before.is_some() {
            Some(canonical_native_slice(
                state, snapshot, source, session, session_id, &messages, order,
                None, CLIENT_MAX_PAGE_ITEMS, managed, true,
            )?.0)
        } else {
            None
        };
        let head = head.as_ref().unwrap_or(&native);
        native_head_fence(head, &source_basis, native.generation)?;
        let count = head.items.len().min(CLIENT_MAX_PAGE_ITEMS);
        conversation_page_header(
            state, session, session_id, &head.items[..count],
            head.has_more || head.items.len() > count,
        )
    } else {
        None
    };
    native.has_more |= native.items.len() > limit + 1;
    native.items.truncate(limit + 1);
    let overlay = crate::profile::span("conversation/overlay");
    // Message sequences are unique, and native_session_messages already returns them
    // newest-first by timestamp and sequence. Compute each full key only once.
    let messages = messages.into_iter().map(|item| {
        let key = crate::external_sessions::timeline_key(&item, 1);
        (item, key)
    }).filter(|(_, key)| before.is_none_or(|before| key.cmp_in(before, order).is_lt()));
    let mut native_items = native.items.into_iter().map(|item| {
        let key = crate::external_sessions::timeline_key(&item, 0);
        (item, key)
    }).peekable();
    let mut messages = messages.peekable();
    let mut items = Vec::with_capacity(limit + 1);
    while items.len() <= limit {
        let take_native = match (native_items.peek(), messages.peek()) {
            (Some((_, native)), Some((_, message))) => native.cmp_in(message, order).is_gt(),
            (Some(_), None) => true,
            // Natives that stopped short of the transcript's end (the echo scan budget) leave
            // unexamined native entries; a graph message below them would be emitted ahead of
            // them and then skipped by the cursor. The next page reads on from the last entry.
            (None, Some(_)) if native.has_more => break,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        items.push(if take_native {
            native_items.next().expect("peeked native entry")
        } else {
            messages.next().expect("peeked message entry")
        });
    }
    let mut has_more = items.len() > limit || native.has_more;
    items.truncate(limit);
    drop(overlay);
    let _page_preparation = crate::profile::span("conversation/page-preparation");
    let receipt_source = crate::external_sessions::external_session_id(source.driver, &source.native_id);
    conversation_delivery::annotate(state, snapshot, session, &receipt_source,
        managed.map(|managed| managed.0.as_str()),
        items.iter_mut().map(|(item, _)| item))?;
    // A page never outgrows what a paired reader can take from a remote owner (its reads are
    // capped at 1 MiB): a seat with very large entries gets a shorter newest page and a cursor
    // for the rest. A lone entry can exceed the target budget, but not the hard ceiling.
    has_more |= bound_page_bytes(&mut items, TIMELINE_PAGE_BYTE_BUDGET);
    let next_cursor = if has_more {
        let boundary = items.last()
            .ok_or_else(|| ApiError::internal("native page has no boundary"))?.1.clone();
        Some(encode_native_page_cursor(NativePagePosition {
            session_id: session_id.to_owned(),
            snapshot: snapshot.clone(),
            collection: format!("timeline/{session_id}"),
            order: order_name.to_owned(),
            boundary,
            limit,
            filter_digest,
            source_basis,
            source_generation: native.generation,
            expires_at_unix_ms,
        })?)
    } else {
        None
    };
    let items: Vec<_> = items.into_iter().rev().map(|(item, _)| item).collect();
    let mut page_value = json!({
        "kind": "timeline-page",
        "session_id": session_id,
        "items": items,
        "page": ClientPageInfo {
            limit, has_more, next_cursor,
            cursor_expires_at: has_more.then(|| client_timestamp(expires_at_unix_ms)),
            omitted_items: None,
        },
    });
    if let Some(header) = header {
        page_value["header"] = header;
    }
    Ok((Json(page_value), native.generation, suppressed_sequence))
}

/// The header's head read must come from the page's own transcript basis and generation.
pub fn native_head_fence(
    head: &conversation_blocks::PreparedSlice, source_basis: &str, generation: u64,
) -> Result<(), ApiError> {
    if head.basis != source_basis || head.generation != generation {
        return Err(client_page_expired("the transcript changed while reading the header"));
    }
    Ok(())
}

/// Refill bounded keyset batches after suppression so an all-echo batch cannot create
/// an empty page or strand older native entries. Every refill retains the original source fence.
/// A call examines at most [`NATIVE_ECHO_SCAN_ITEMS`] entries: past that it returns the visible
/// entries found so far (a short page whose boundary is its last entry, so the next page reads
/// on with a fresh budget), and refuses (`page-cursor-expired`) only when that many consecutive
/// entries are all hidden.
#[allow(clippy::too_many_arguments)]
pub fn canonical_native_slice(
    state: &AppState,
    snapshot: &ClientSnapshot,
    source: &crate::external_sessions::ExternalSession,
    session: &ClientSession,
    session_id: &str,
    graph: &[Value],
    order: crate::external_sessions::TimelineOrder,
    before: Option<&crate::external_sessions::TimelineKey>,
    limit: usize,
    managed: Option<&super::ManagedSessionOwner>,
    prepare_display: bool,
) -> Result<(conversation_blocks::PreparedSlice, u64), ApiError> {
    let mut slice = conversation_blocks::read_slice(
        source, session, session_id, order, before, limit, prepare_display,
    )?;
    let mut boundary = slice.items.last().map(|item| crate::external_sessions::timeline_key(item, 0));
    let mut scanned = slice.items.len();
    let mut suppressed = suppress_native_echoes(state, snapshot, session_id, graph, &mut slice.items, managed)?;
    while slice.items.len() < limit && slice.has_more {
        let budget = NATIVE_ECHO_SCAN_ITEMS.saturating_sub(scanned);
        if budget == 0 && !slice.items.is_empty() {
            break;
        }
        if budget == 0 {
            // Not cursor-gap: that code asks the client to reload the newest page, which meets
            // the same hidden prefix and the same error, so older history would be unreachable.
            return Err(client_page_expired(
                "more consecutive delivery echoes than one bounded read examines hide the next native entries; the page cannot continue",
            ));
        }
        let mut next = conversation_blocks::read_slice(
            source, session, session_id, order, boundary.as_ref(),
            (limit - slice.items.len()).min(budget),
            prepare_display,
        )?;
        scanned += next.items.len();
        if next.basis != slice.basis || next.generation != slice.generation {
            return Err(client_page_expired("the transcript changed while refilling the page"));
        }
        boundary = next.items.last().map(|item| crate::external_sessions::timeline_key(item, 0));
        suppressed = suppressed.max(suppress_native_echoes(
            state, snapshot, session_id, graph, &mut next.items, managed,
        )?);
        slice.has_more = next.has_more;
        slice.items.extend(next.items);
    }
    Ok((slice, suppressed))
}

/// The most native entries one echo-omitting read examines.
#[cfg(not(any(test, feature = "test-support")))]
pub const NATIVE_ECHO_SCAN_ITEMS: usize = 4096;
/// Test builds (fractal-api-client-v0's tests reach this through test-support) scan a smaller budget, so a
/// fixture can exceed it with a few hundred echoes.
#[cfg(any(test, feature = "test-support"))]
pub const NATIVE_ECHO_SCAN_ITEMS: usize = 256;
/// The most a timeline page's entries may serialize to, leaving room under a peer's 1 MiB cap on
/// a remote client read for the envelope and the page header.
pub const TIMELINE_PAGE_BYTE_BUDGET: usize = 768 * 1024;

/// Replace a timeline entry that alone exceeds the response ceiling with a visible error entry
/// that keeps its identity and place, so the page it is on still opens and continues past it.
pub fn cut_oversized_timeline_entry(item: &mut Value, size: usize) {
    item["type"] = json!("error");
    item["role"] = json!("system");
    item["body"] = json!({
        "code": "timeline-entry-too-large",
        "message": format!("[st truncated this timeline entry: size limit; {size} bytes]"),
        "retryable": false,
        "details": {"size": size},
    });
}

/// Keep complete newest entries within the target budget. One larger entry may be served
/// alone up to the hard response ceiling; one above the ceiling is replaced, keeping its
/// identity and place, by a visible error entry that says it was cut. Continuation uses the
/// actual last returned key.
pub fn bound_page_bytes<K>(items: &mut Vec<(Value, K)>, budget: usize) -> bool {
    let ceiling = CLIENT_MAX_RESPONSE_BYTES - 128_000;
    let mut bytes = 2usize; // JSON array delimiters.
    for index in 0..items.len() {
        let size = serde_json::to_vec(&items[index].0).map_or(0, |encoded| encoded.len());
        bytes = bytes
            .saturating_add(size)
            .saturating_add(usize::from(index > 0));
        if bytes > budget {
            if index > 0 {
                items.truncate(index);
                return true;
            }
            if bytes > ceiling {
                cut_oversized_timeline_entry(&mut items[0].0, size);
            }
            let more = items.len() > 1;
            items.truncate(1);
            return more;
        }
    }
    false
}

pub fn post_snapshot_native_sends(
    state: &AppState,
    snapshot: &ClientSnapshot,
    owner: &str,
    session_id: &str,
    incarnation: Option<&str>,
    items: &[Value],
    incoming: &BTreeSet<&str>,
) -> Result<Vec<ClaimRecord>, ApiError> {
    if !items.iter().any(|item| matches!(
        item["_delivery_correlation"]["kind"].as_str(), Some("message" | "codex")
    )) {
        return Ok(Vec::new());
    }
    let through = state.store.index().map_err(ApiError::internal)?;
    if through <= snapshot.store_index {
        return Ok(Vec::new());
    }
    let (started, ended) = match incarnation {
        Some(incarnation) => state.store.conversation_runtime_span(owner, incarnation)
            .map_err(ApiError::internal)?,
        None => (None, None),
    };
    let mut future = if items.iter().any(|item| item["_delivery_correlation"]["kind"] == "codex") {
        let delta = state.store.conversation_message_delta_at(
            owner, through.checked_add(1), snapshot.store_index, 256,
        ).map_err(ApiError::internal)?;
        let subjects = incoming.iter().copied()
            .chain(delta.iter().filter_map(|claim| {
                let fields = claim.body.get("fields").unwrap_or(&claim.body);
                (session_message_belongs(claim, owner, session_id, started, ended)
                    && fields["to"].as_str() == Some(owner)
                    && fields["from"].as_str() != Some(owner))
                    .then_some(claim.subject.as_str())
            })).collect::<BTreeSet<_>>();
        let unresolved = items.iter().filter_map(|item| item.get("_delivery_correlation"))
            .filter(|correlation| correlation["kind"] == "codex")
            .any(|correlation| {
                let Some(thread) = correlation["thread_id"].as_str() else { return true };
                let Some(id) = correlation["id"].as_str() else { return true };
                !subjects.iter().any(|subject| {
                    fractal_drivers::codex_app_server::stable_client_user_message_id(owner, thread, subject) == id
                })
            });
        if delta.len() == 256 && unresolved {
            return Err(ApiError {
                status: StatusCode::GONE, code: "cursor-gap".into(),
                message: "the native read crossed too many graph sends; refresh the conversation".into(),
                details: Box::new(serde_json::Map::from_iter([("full_resync".into(), json!(true))])),
            });
        }
        delta
    } else {
        Vec::new()
    };
    let subjects = items.iter()
        .filter_map(|item| item.get("_delivery_correlation"))
        .filter(|correlation| correlation["kind"] == "message")
        .filter_map(|correlation| correlation["id"].as_str())
        .collect::<BTreeSet<_>>();
    for subject in subjects {
        if incoming.contains(subject) || future.iter().any(|claim| claim.subject == subject) {
            continue;
        }
        if let Some(claim) = state.store.latest_claim(subject, Some("message.sent"))
            .map_err(ApiError::internal)?
            && claim.store_index > snapshot.store_index
        {
            future.push(claim);
        }
    }
    future.retain(|claim| session_message_belongs(claim, owner, session_id, started, ended));
    Ok(future)
}

pub fn native_echo_matches(
    correlation: &Value,
    incoming: &BTreeSet<&str>,
    codex_ids: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    match correlation["kind"].as_str() {
        Some("message") => correlation["id"].as_str().is_some_and(|id| incoming.contains(id)),
        Some("codex") => correlation["thread_id"].as_str()
            .and_then(|thread| codex_ids.get(thread))
            .is_some_and(|ids| correlation["id"].as_str().is_some_and(|id| ids.contains(id))),
        _ => false,
    }
}

/// What st3 established about a managed seat's native transcript.
pub struct ManagedTranscript {
    /// The harness the seat's latest observation names.
    pub driver: String,
    /// The `harness.observed` claim the verdict rests on. A notice about a missing transcript is
    /// placed beside it in the timeline, so it moves forward when the observation changes.
    anchor: ClaimRecord,
    /// The seat's exact native session, or why st3 could not bind one.
    pub transcript: Result<crate::external_sessions::ExternalSession, Missing>,
}

/// Why a seat's transcript is not shown. `not_yet` means the harness has not written one for
/// this incarnation: the seat has said nothing since it started, which is not a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    pub reason: String,
    pub not_yet: bool,
}

impl Missing {
    fn not_yet(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            not_yet: true,
        }
    }
}

impl From<String> for Missing {
    fn from(reason: String) -> Self {
        Self {
            reason,
            not_yet: false,
        }
    }
}

impl From<&str> for Missing {
    fn from(reason: &str) -> Self {
        reason.to_owned().into()
    }
}

/// Bind a managed seat to its exact native transcript, when its harness keeps one st3 reads.
///
/// `None` means the seat runs no such harness (or has not reported one), and its timeline is
/// built from claims alone as before. Otherwise the result either carries the exact session or
/// says why none could be bound, so the timeline can say so rather than silently showing only
/// status entries. The binding is never widened to a guess: a transcript that cannot be tied to
/// this seat's current incarnation is not shown.
pub fn managed_transcript(
    state: &AppState,
    owner: &str,
    incarnation: &str,
) -> Result<Option<ManagedTranscript>, ApiError> {
    let Some(anchor) = state
        .store
        .latest_claim(owner, Some("harness.observed"))
        .map_err(ApiError::internal)?
    else {
        return Ok(None);
    };
    let fields = anchor.body.get("fields").unwrap_or(&anchor.body);
    let driver = fields["driver"].as_str().unwrap_or_default().to_owned();
    if !matches!(driver.as_str(), "codex" | "claude" | "omp" | "pi") {
        return Ok(None);
    }
    let transcript = if fields["incarnation_id"] != incarnation {
        Err(Missing::not_yet(
            "the harness has not reported on the seat's current incarnation yet",
        ))
    } else {
        let evidence = fields["evidence_incarnation"].as_str();
        match driver.as_str() {
            "codex" => managed_codex_transcript(state, owner, evidence),
            "claude" => managed_claude_transcript(state, owner, evidence),
            _ => managed_pi_family_transcript(
                state,
                owner,
                incarnation,
                if driver == "pi" {
                    crate::external_sessions::ExternalDriver::Pi
                } else {
                    crate::external_sessions::ExternalDriver::Omp
                },
            ),
        }
    };
    // A seat that started moments ago has not had time to bind a transcript, whichever piece is
    // missing yet: that is starting up, not a fault, and a person watching a new agent is told so
    // calmly instead of "could not be loaded".
    let transcript = match transcript {
        Err(missing) if !missing.not_yet && started_recently(incarnation) => {
            Err(Missing::not_yet(missing.reason))
        }
        other => other,
    };
    Ok(Some(ManagedTranscript {
        driver,
        anchor,
        transcript,
    }))
}

/// How long after a seat's incarnation starts a missing transcript binding is still startup.
const TRANSCRIPT_STARTUP_GRACE_MS: i64 = 120_000;

/// Whether `incarnation` (`PID:START`, START an RFC 3339 time) started within the startup grace.
pub fn started_recently(incarnation: &str) -> bool {
    incarnation
        .split_once(':')
        .and_then(|(_, started)| chrono::DateTime::parse_from_rfc3339(started).ok())
        .is_some_and(|started| {
            let age = chrono::Utc::now().timestamp_millis() - started.timestamp_millis();
            (0..TRANSCRIPT_STARTUP_GRACE_MS).contains(&age)
        })
}

/// The timeline entry that says a managed seat's native transcript is not shown, and why.
/// A timeline entry saying why the seat's transcript is not shown. When st3 bound the transcript
/// but could not read it, the entry names the file, so the failure can be reported.
fn transcript_notice(session_id: &str, managed: &ManagedTranscript, reason: &str) -> Value {
    let anchor = &managed.anchor;
    let mut details = json!({ "driver": managed.driver, "claim_id": anchor.id, "availability":"transcript-unavailable" });
    match &managed.transcript {
        Ok(external) => {
            details["transcript"] = Value::String(external.transcript.display().to_string());
        }
        // Nothing has gone wrong: the seat has said nothing since it started.
        Err(missing) if missing.not_yet => details["not_yet"] = Value::Bool(true),
        Err(_) => {}
    }
    let fields = anchor.body.get("fields").unwrap_or(&anchor.body);
    let digest = hex::encode(Sha256::digest(
        format!("{}:transcript-not-bound", anchor.id).as_bytes(),
    ));
    json!({
        "id": format!("timeline-entry/{}/{}", session_id.trim_start_matches("session/"), &digest[..24]),
        // Slot 2 of the observation's four sequence slots is otherwise unused.
        "sequence": anchor.store_index.saturating_mul(4).saturating_add(2),
        "revision": 1,
        "timestamp": client_timestamp(
            fields
                .get("observed_at_unix_ms")
                .and_then(Value::as_u64)
                .map(u128::from)
                .unwrap_or(anchor.accepted_at_unix_ms),
        ),
        "role": "system",
        "type": "error",
        "final": true,
        "body": {
            "code": "transcript-not-bound",
            "message": format!("transcript not bound: {reason}"),
            "retryable": true,
            "details": details
        }
    })
}

fn managed_codex_transcript(
    state: &AppState,
    owner: &str,
    evidence: Option<&str>,
) -> Result<crate::external_sessions::ExternalSession, Missing> {
    let Some(home) = state.native_session_home.as_deref() else {
        return Err("this daemon has no home directory to read native sessions from".into());
    };
    // The wrapper owns this path; never resolve a path from client input. A reused
    // driver directory is only authoritative when its runtime and a durable
    // observation both name the same exact provider incarnation.
    let root = state
        .state_dir
        .join("drivers")
        .join(&hex::encode(Sha256::digest(owner.as_bytes()))[..24]);
    let native = root.join("sessions/codex");
    let directory = if native.exists() {
        native
    } else {
        root.join("state")
    };
    let runtime = std::fs::read(directory.join("runtime.json"))
        .map_err(|_| "the Codex driver has not written its runtime record".to_owned())?;
    let binding = std::fs::read(directory.join("binding.json"))
        .map_err(|_| "the Codex driver has not bound a thread yet".to_owned())?;
    let (Ok(runtime), Ok(binding)) = (
        serde_json::from_slice::<Value>(&runtime),
        serde_json::from_slice::<Value>(&binding),
    ) else {
        return Err("the Codex driver's runtime or binding record is unreadable".into());
    };
    let identity = owner.strip_prefix("agent/").unwrap_or(owner);
    let Some(provider_incarnation) = runtime["incarnation"].as_str() else {
        return Err("the Codex runtime record names no incarnation".into());
    };
    let Some(native_id) = binding["threadId"].as_str() else {
        return Err("the Codex binding names no thread".into());
    };
    if runtime["agent"] != identity
        || binding["agent"] != identity
        || binding["runtimeIncarnation"] != provider_incarnation
    {
        return Err("the Codex binding belongs to a different runtime".into());
    }
    if evidence != Some(provider_incarnation) {
        return Err("the Codex binding is from a different provider incarnation".into());
    }
    // Codex names the rollout after its thread, so look it up directly first; a rollout whose
    // name does not follow that convention is still found by the thread ID inside it.
    let bound = match crate::external_sessions::find_bound_transcript(
        home,
        crate::external_sessions::ExternalDriver::Codex,
        native_id,
    ) {
        Ok(Some(session)) => Some(session),
        _ => crate::external_sessions::discover(Some(home), true)
            .map_err(|error| format!("listing Codex sessions failed: {error:#}"))?
            .sessions
            .into_iter()
            .find(|session| {
                session.driver == crate::external_sessions::ExternalDriver::Codex
                    && session.native_id == native_id
            }),
    };
    bound.ok_or_else(|| {
        Missing::not_yet(format!("Codex thread {native_id} has no rollout file yet"))
    })
}

fn managed_claude_transcript(
    state: &AppState,
    owner: &str,
    evidence: Option<&str>,
) -> Result<crate::external_sessions::ExternalSession, Missing> {
    let Some(home) = state.native_session_home.as_deref() else {
        return Err("this daemon has no home directory to read native sessions from".into());
    };
    let Some(evidence) = evidence else {
        return Err("the Claude driver has not reported which process owns the seat".into());
    };
    let directory = crate::hooks::claude_agent_dir(
        &state.state_dir.join("drivers"),
        owner,
        &fractal_drivers::run::detect_host(),
    );
    // The current wrapper's SessionStart hook binds the Claude session it started. A previous
    // provider's binding can survive a restart, so it counts only when it names the same
    // provider incarnation as the seat's current observation; neither its presence nor the
    // newest transcript in a workspace is sufficient.
    let hook_record = std::fs::read(directory.join("claude-native-session"))
        .ok()
        .and_then(|binding| serde_json::from_slice::<Value>(&binding).ok())
        .filter(|binding| binding["incarnation"].as_str() == Some(evidence));
    let hook_binding = hook_record
        .as_ref()
        .and_then(|binding| binding["native_session_id"].as_str().map(str::to_owned));
    // The path Claude gave the hook is where this session's transcript really is, on whatever
    // login the seat runs: `~/.claude` or any account's `CLAUDE_CONFIG_DIR`.
    let recorded_path = hook_record
        .as_ref()
        .and_then(|binding| binding["transcript_path"].as_str())
        .map(std::path::PathBuf::from);
    let native_id = match hook_binding {
        Some(native_id) => native_id,
        // Without the hook, prove the session from the live processes instead; see
        // `claude_session_of_managed_driver` for why that cannot pick another session.
        None => crate::external_sessions::claude_session_of_managed_driver(home, owner, evidence)
            .map_err(|reason| {
            format!("the SessionStart hook did not bind this incarnation, and {reason}")
        })?,
    };
    let find = |extra_roots: &[std::path::PathBuf]| {
        crate::external_sessions::find_bound_transcript_with(
            home,
            crate::external_sessions::ExternalDriver::Claude,
            &native_id,
            recorded_path.as_deref(),
            extra_roots,
        )
    };
    let mut found = find(&[]);
    // A seat started before its binding recorded a path, on an account's own login: look in the
    // declared accounts' login directories too. Only reached when the ordinary places miss.
    if matches!(found, Ok(None)) {
        let host = fractal_drivers::run::detect_host();
        let roots = state
            .store
            .declared_accounts()
            .unwrap_or_default()
            .iter()
            .filter(|account| account.driver() == Some("claude"))
            .filter_map(|account| account.login_for(&host))
            .filter_map(|login| crate::accounts::expand_login(login, Some(home)))
            .map(|login| login.join("projects"))
            .collect::<Vec<_>>();
        if !roots.is_empty() {
            found = find(&roots);
        }
    }
    match found {
        Ok(Some(session)) => Ok(session),
        Ok(None) => Err(Missing::not_yet(format!(
            "Claude session {native_id} has no transcript file yet"
        ))),
        Err(error) => Err(format!("finding Claude session {native_id} failed: {error:#}").into()),
    }
}

fn pi_family_binding<'a>(
    bindings: &'a [crate::model::ClaimRecord],
    owner: &str,
    incarnation: &str,
    driver: crate::external_sessions::ExternalDriver,
) -> Option<&'a crate::model::ClaimRecord> {
    let matches_owner = |fields: &Value| {
        fields["harness"] == driver.as_str() && fields["agent"] == owner
    };
    bindings.iter().rev().find(|claim| {
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        matches_owner(fields) && fields["incarnation_id"] == incarnation
    }).or_else(|| bindings.last().filter(|claim| {
        // Import provenance is usable only until a runtime binding supersedes it.
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        matches_owner(fields)
            && fields["incarnation_id"].is_null()
            && fields["source_session"].as_str().is_some()
    }))
}

#[cfg(test)]
mod pi_family_binding_tests {
    use super::*;

    #[test]
    fn exact_incarnation_beats_newer_imports_and_other_runtime_bindings() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&root.path().join("graph.db"), "test").unwrap();
        let owner = "agent/binding-test";
        let append = |incarnation: Option<&str>, source: Option<&str>, agent: &str| {
            if let Some(incarnation) = incarnation {
                store.append_claim(&crate::model::ClaimInput {
                    subject: owner.into(), kind: "runtime.observed".into(),
                    actor: Some("person/robin".into()),
                    fields: BTreeMap::from([
                        ("status".into(), json!("running")),
                        ("incarnation_id".into(), json!(incarnation)),
                    ]),
                    evidence: Vec::new(), expected_subject: None, idempotency_key: None,
                }).unwrap();
            }
            store.append_claim(&crate::model::ClaimInput {
                subject: owner.into(),
                kind: "harness.session-file".into(),
                actor: Some("person/robin".into()),
                fields: BTreeMap::from([
                    ("harness".into(), json!("omp")),
                    ("session_id".into(), json!("native-binding")),
                    ("agent".into(), json!(agent)),
                    ("incarnation_id".into(), json!(incarnation)),
                    ("source_session".into(), json!(source)),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            }).unwrap()
        };
        let imported = append(None, Some("session/imported"), owner);
        let claims = || store.claims_for(owner, Some("harness.session-file")).unwrap();
        let selected = |bindings: &[crate::model::ClaimRecord], incarnation: &str| {
            pi_family_binding(bindings, owner, incarnation, crate::external_sessions::ExternalDriver::Omp)
                .map(|claim| claim.id.clone())
        };
        assert_eq!(selected(&claims(), "old"), Some(imported.id));
        let exact = append(Some("old"), None, owner);
        append(Some("new"), None, owner);
        assert_eq!(selected(&claims(), "old"), Some(exact.id.clone()));
        assert_eq!(selected(&claims(), "unbound"), None, "runtime claims supersede import provenance");
        append(None, Some("session/later-import"), owner);
        assert_eq!(selected(&claims(), "old"), Some(exact.id.clone()));
        append(Some("old"), None, "agent/other-owner");
        assert_eq!(selected(&claims(), "old"), Some(exact.id));
        assert_eq!(selected(&claims(), "unbound"), None, "a foreign latest claim cannot revive an import");
    }
}

fn managed_pi_family_transcript(
    state: &AppState,
    owner: &str,
    incarnation: &str,
    driver: crate::external_sessions::ExternalDriver,
) -> Result<crate::external_sessions::ExternalSession, Missing> {
    let bindings = state
        .store
        .claims_for(owner, Some("harness.session-file"))
        .map_err(|error| format!("reading the pi-family session record failed: {error:#}"))?;
    if let Some(claim) = pi_family_binding(&bindings, owner, incarnation, driver) {
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        let (Some(path), Some(native_id)) =
            (fields["path"].as_str(), fields["session_id"].as_str())
        else {
            return Err(Missing::not_yet(
                "the bound pi-family session has no transcript path",
            ));
        };
        return match crate::external_sessions::find_bound_pi_family_transcript(
            driver,
            Path::new(path),
            native_id,
        ) {
            Ok(Some(session)) => Ok(session),
            Ok(None) => Err(format!(
                "the bound {} session {native_id} is not readable",
                driver.as_str()
            )
            .into()),
            Err(error) => Err(format!(
                "reading the bound {} session {native_id} failed: {error:#}",
                driver.as_str()
            )
            .into()),
        };
    }
    let root = state
        .state_dir
        .join("drivers")
        .join(&hex::encode(Sha256::digest(owner.as_bytes()))[..24]);
    let native = root
        .join("sessions")
        .join(driver.as_str())
        .join("provider-sessions");
    let directory = if root.join("sessions").join(driver.as_str()).exists() {
        native
    } else {
        crate::hooks::legacy_claude_agent_dir(
            &state.state_dir.join("drivers"),
            owner,
            &fractal_drivers::run::detect_host(),
        )
        .join("provider-sessions")
    };
    if driver == crate::external_sessions::ExternalDriver::Pi
        || std::fs::symlink_metadata(&directory)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Missing::not_yet(
            "the pi-family channel has not bound this incarnation's transcript",
        ));
    }
    let started_at = incarnation
        .split_once(':')
        .and_then(|(_, started_at)| chrono::DateTime::parse_from_rfc3339(started_at).ok())
        .ok_or_else(|| {
            format!("the OMP incarnation `{incarnation}` does not carry its start time")
        })?;
    match crate::external_sessions::find_managed_omp_transcript(
        &directory,
        (started_at.timestamp_millis().max(0) as u128).saturating_sub(2_000),
    ) {
        Ok(Some(session)) => Ok(session),
        Ok(None) => Err(Missing::not_yet(
            "OMP has not saved a session for this incarnation yet",
        )),
        Err(error) => Err(format!("reading the OMP session directory failed: {error:#}").into()),
    }
}

pub fn external_conversation_items(
    conversation: Option<crate::external_sessions::ExternalConversation>,
    session_id: &str,
) -> Result<Vec<Value>, ApiError> {
    match conversation {
        Some(crate::external_sessions::ExternalConversation::Readable(external)) => {
            crate::external_sessions::normalized_timeline(&external).map_err(ApiError::internal)
        }
        Some(crate::external_sessions::ExternalConversation::Unavailable(process)) => Err(ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "unsupported-capability".into(),
            message: "This agent was not started by st, and st could not identify its saved session. Its conversation is not available.".into(),
            details: Box::new(serde_json::Map::from_iter([
                ("session_id".into(), json!(process.id)),
                ("reason".into(), json!("native-session-unidentified")),
            ])),
        }),
        None => Err(ApiError::not_found(format!(
            "session `{session_id}` does not exist"
        ))),
    }
}

pub fn timeline_value(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    id: &str,
    query: &ClientListQuery,
) -> Result<Json<Value>, ApiError> {
    require_scope(session, "read.projections")?;
    let session_id = client_detail_id("session", id);
    if let Some(cursor) = query.cursor.as_deref() {
        let _span = crate::profile::span("timeline/cached-page");
        if cursor.starts_with(NATIVE_PAGE_CURSOR_PREFIX) {
            let managed = super::managed_session_owner_at(&state.store, snapshot.store_index, &session_id)
                .map_err(ApiError::internal)?;
            let source = conversation_blocks::source(state, &session_id, managed.as_ref())
                .map_err(|_| client_page_expired("the page snapshot is no longer available; restart pagination"))?;
            return native_slice_page_at_generation(
                state, snapshot, session, &session_id, query, &source, managed.as_ref(),
            ).map(|(page, _, _)| page);
        }
        let mut page = client_page(
            state,
            snapshot,
            &format!("timeline/{session_id}"),
            Vec::new(),
            query,
        )?;
        let header = query
            .cursor
            .as_deref()
            .and_then(|cursor| offset_continuation_header(state, session, &session_id, snapshot, cursor));
        for item in &mut page.items {
            conversation_participants::legacy_item(item, session);
            if !session.conversation_delivery_links && let Some(item) = item.as_object_mut() { item.remove("delivers"); }
            if !session.conversation_blocks && let Some(body) = item["body"].as_object_mut() {
                body.remove("blocks");
            }
        }
        page.items.reverse();
        let mut page_value = json!({
            "kind": "timeline-page",
            "session_id": session_id,
            "items": page.items,
            "page": page.page
        });
        if let Some(header) = header {
            page_value["header"] = header;
        }
        return Ok(Json(page_value));
    }
    crate::performance::task("conversation/first-page", || {
        let managed = super::managed_session_owner_at(&state.store, snapshot.store_index, &session_id)
            .map_err(ApiError::internal)?;
        timeline_first_page(state, snapshot, session, session_id, query, None, managed.as_ref())
    })
}

/// Stable identity shared by stored projection and conversation replay selection.
pub fn timeline_claim_entry_id(session_id: &str, claim: &ClaimRecord, suffix: &str) -> String {
    let digest = hex::encode(Sha256::digest(format!("{}:{suffix}", claim.id).as_bytes()));
    format!("timeline-entry/{}/{}", session_id.trim_start_matches("session/"), &digest[..24])
}

pub fn timeline_first_page(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    session_id: String,
    query: &ClientListQuery,
    native_read_error: Option<ApiError>,
    managed: Option<&super::ManagedSessionOwner>,
) -> Result<Json<Value>, ApiError> {
    timeline_first_page_with_extent(state, snapshot, session, session_id, query, native_read_error, managed, None)
}

pub type TimelineExtentVisitor<'a> = &'a mut dyn FnMut(&[Value]);

/// Capture the complete bounded stored projection beside its display window for replay loss checks.
#[allow(clippy::too_many_arguments)]
pub fn timeline_first_page_with_extent(
    state: &AppState,
    snapshot: &ClientSnapshot,
    session: &ClientSession,
    session_id: String,
    query: &ClientListQuery,
    native_read_error: Option<ApiError>,
    managed: Option<&super::ManagedSessionOwner>,
    extent: Option<TimelineExtentVisitor<'_>>,
) -> Result<Json<Value>, ApiError> {
    #[cfg(any(test, feature = "test-support"))]
    if let Ok(mut rebuilds) = timeline_rebuilds().lock() {
        *rebuilds.entry(session_id.to_owned()).or_default() += 1;
    }
    let Some((owner, incarnation, _)) = managed else {
        if let Some(error) = native_read_error {
            return Err(error);
        }
        let conversation = crate::external_sessions::find_conversation(
            state.native_session_home.as_deref(),
            &session_id,
        )
        .map_err(ApiError::internal)?;
        let items = match conversation {
            Some(crate::external_sessions::ExternalConversation::Readable(external)) => {
                let _span = crate::profile::span("timeline/native-read");
                if external.driver != crate::external_sessions::ExternalDriver::OpenCode {
                    return native_slice_page(state, snapshot, session, &session_id, query, &external);
                }
                conversation_blocks::read(&external, session, &session_id)?
            }
            _ if crate::subagent_sessions::child_parts(&session_id).is_some() => {
                let child = conversation_blocks::source(state, &session_id, None)?;
                if child.driver != crate::external_sessions::ExternalDriver::OpenCode {
                    return native_slice_page(state, snapshot, session, &session_id, query, &child);
                }
                conversation_blocks::read(&child, session, &session_id)?
            }
            other => external_conversation_items(other, &session_id)?,
        };
        return native_timeline_page(state, snapshot, session, &session_id, query, items, managed);
    };
    let owner = owner.as_str();
    let incarnation = incarnation.as_deref();
    // When the seat's harness keeps a transcript st3 cannot bind or read, the claim timeline
    // below is shown with one entry that says why, never silently in its place.
    let mut transcript_notice_entry = None;
    if let Some(incarnation) = incarnation
        && let Some(transcript) = managed_transcript(state, owner, incarnation)?
    {
        let read = if let Some(error) = native_read_error.as_ref() {
            Err(format!("the transcript could not be read: {}", error.message))
        } else {
            let _span = crate::profile::span("timeline/native-read");
            match transcript.transcript.as_ref() {
                Ok(external) => match if external.driver != crate::external_sessions::ExternalDriver::OpenCode {
                    native_slice_page(state, snapshot, session, &session_id, query, external)
                } else {
                    conversation_blocks::read(external, session, &session_id)
                        .and_then(|items| native_timeline_page(state, snapshot, session, &session_id, query, items, managed))
                } {
                    Ok(page) => Ok(page),
                    Err(error) if error.status == StatusCode::TOO_MANY_REQUESTS || error.code == "cursor-gap" => return Err(error),
                    Err(error) => Err(format!("the transcript could not be read: {}", error.message)),
                },
                Err(missing) => Err(missing.reason.clone()),
            }
        };
        match read {
            Ok(page) => return Ok(page),
            Err(reason) => {
                transcript_notice_entry = Some(transcript_notice(&session_id, &transcript, &reason));
            }
        }
    }
    let _fallback_span = crate::profile::span("timeline/stored-fallback");
    let desired = state.store.desired_subjects_named(&[owner.to_owned()])
        .map_err(ApiError::internal)?;
    let attribution = timeline_attribution(owner, &desired);
    let before = snapshot.store_index.checked_add(1);
    let timeline_page = if let Some(incarnation) = incarnation {
        state
            .store
            .timeline_claims_for_incarnation_at(owner, incarnation, before, true, 4_096)
            .map_err(ApiError::internal)?
    } else {
        crate::model::ClaimsPage {
            claims: Vec::new(),
            next_cursor: None,
        }
    };
    let has_older_timeline = timeline_page.next_cursor.is_some();
    let mut timeline_claims = timeline_page.claims;
    timeline_claims.reverse();
    timeline_claims.retain(|claim| {
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        incarnation.is_some_and(|expected| {
            fields.get("incarnation_id").and_then(Value::as_str) == Some(expected)
        })
    });
    // The oldest physical row, not a query cursor, establishes missing prefix history.
    let oldest = if let Some(incarnation) = incarnation {
        state.store.timeline_claims_for_incarnation_at(owner, incarnation, before, false, 1)
            .map_err(ApiError::internal)?.claims.pop()
    } else {
        None
    };
    let missing_prefix_through = oldest.as_ref().and_then(|claim| {
        claim.body.pointer("/fields/sequence").and_then(Value::as_u64)
    }).unwrap_or(1).saturating_sub(1);
    let prefix_unavailable = if let Some(incarnation) = incarnation {
        !state.store.timeline_retention_is_explicit_at(
            owner, incarnation, before, missing_prefix_through,
        ).map_err(ApiError::internal)?
    } else {
        false
    };
    let window_start = timeline_claims.first().map(crate::store::claim_log_order);
    let missing_appends = {
        let mut retained_entries = BTreeSet::new();
        let mut missing = BTreeSet::new();
        for claim in &timeline_claims {
            let fields = claim.body.get("fields").unwrap_or(&claim.body);
            let Some(entry_id) = fields.get("entry_id").and_then(Value::as_str) else {
                continue;
            };
            if retained_entries.insert(entry_id)
                && fields.get("operation").and_then(Value::as_str) != Some("append")
            {
                missing.insert(entry_id.to_owned());
            }
        }
        missing
    };
    let appends = if has_older_timeline && !missing_appends.is_empty() {
        state.store.timeline_entry_append_positions_at(
            owner, incarnation.unwrap_or_default(), &missing_appends, before,
        ).map_err(ApiError::internal)?
    } else {
        BTreeMap::new()
    };
    let mut omitted_updates = BTreeSet::new();
    for entry_id in missing_appends {
        if appends.get(&entry_id).is_some_and(|append| {
            window_start.is_some_and(|start| *append < start)
        }) {
            // An entry outside this view is not absent from retained storage.
            // Never fabricate its latest revision without intervening operations.
            omitted_updates.insert(entry_id);
            continue;
        }
        return Err(ApiError {
            status: StatusCode::GONE,
            code: "timeline-history-incomplete".into(),
            message: "the retained transcript start is incomplete: an entry's append operation is missing".into(),
            details: Box::new(serde_json::Map::from_iter([
                ("full_resync".into(), Value::Bool(false)),
                ("retained_history_incomplete".into(), Value::Bool(true)),
            ])),
        });
    }
    timeline_claims.retain(|claim| {
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        !fields.get("entry_id").and_then(Value::as_str)
            .is_some_and(|entry| omitted_updates.contains(entry))
    });
    let mut owner_claims = state
        .store
        .conversation_timeline_owner_claims_at(owner, before)
        .map_err(ApiError::internal)?;
    owner_claims.reverse();
    let mut message_claims = session_messages(state, owner, &session_id, incarnation, before)?;
    message_claims.reverse();
    let mut claims = timeline_claims;
    claims.extend(owner_claims);
    claims.extend(message_claims);
    claims.sort_by_key(crate::store::claim_log_order);
    claims.dedup_by_key(|claim| claim.id.clone());
    let session_leaf = session_id.trim_start_matches("session/");
    let mut items = Vec::<Value>::new();
    let mut explicit = BTreeMap::<String, usize>::new();
    let mut tool_calls = BTreeSet::<String>::new();
    let entry_id = |claim: &ClaimRecord, suffix: &str| timeline_claim_entry_id(&session_id, claim, suffix);
    let applies_to_incarnation = |fields: &Value| {
        let observed = fields.get("incarnation_id").and_then(Value::as_str);
        observed.is_none() || incarnation.is_none() || observed == incarnation
    };
    for claim in claims {
        let fields = claim.body.get("fields").unwrap_or(&claim.body);
        let timestamp = client_timestamp(
            fields
                .get("observed_at_unix_ms")
                .and_then(Value::as_u64)
                .map(u128::from)
                .unwrap_or(claim.accepted_at_unix_ms),
        );
        let base_sequence = claim.store_index.saturating_mul(4);
        if claim.subject == owner && claim.kind == "harness.timeline" {
            if !incarnation.is_some_and(|expected| {
                fields.get("incarnation_id").and_then(Value::as_str) == Some(expected)
            }) {
                continue;
            }
            let Some(operation) = fields.get("operation").and_then(Value::as_str) else {
                continue;
            };
            let Some(id) = fields.get("entry_id").and_then(Value::as_str) else {
                continue;
            };
            let revision = fields.get("revision").and_then(Value::as_u64).unwrap_or(1);
            let role = fields
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("system");
            let entry_type = fields
                .get("entry_type")
                .and_then(Value::as_str)
                .unwrap_or("error");
            let final_entry = fields
                .get("final")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let sequence = fields
                .get("sequence")
                .and_then(Value::as_u64)
                .unwrap_or(base_sequence + 3);
            let mut body = fields.get("body").cloned().unwrap_or_else(|| json!({}));
            if entry_type == "usage" {
                body = normalized_timeline_usage_body(
                    body,
                    fields.get("attribution").unwrap_or(&attribution),
                    fields.get("driver").and_then(Value::as_str),
                );
            }
            let transition_valid = match operation {
                "append" => !explicit.contains_key(id) && revision == 1,
                "replace" | "finalize" => explicit.get(id).is_some_and(|index| {
                    let current = &items[*index];
                    !current["final"].as_bool().unwrap_or(false)
                        && current["revision"].as_u64().unwrap_or(0) + 1 == revision
                        && current["role"] == role
                        && current["type"] == entry_type
                }),
                _ => false,
            };
            let tool_order_valid = if entry_type == "tool_result" {
                body.get("call_id")
                    .and_then(Value::as_str)
                    .is_some_and(|call_id| tool_calls.contains(call_id))
            } else {
                true
            };
            if !transition_valid || !tool_order_valid {
                items.push(json!({
                    "id": entry_id(&claim, "invalid-transition"),
                    "sequence": sequence,
                    "revision": 1,
                    "timestamp": timestamp,
                    "role": "system",
                    "type": "error",
                    "final": true,
                    "body": {
                        "code": "invalid-timeline-transition",
                        "message": format!("driver timeline entry `{id}` has an invalid {operation} transition"),
                        "retryable": false,
                        "details": { "claim_id": claim.id }
                    }
                }));
                continue;
            }
            if entry_type == "tool_call"
                && let Some(call_id) = body.get("call_id").and_then(Value::as_str)
            {
                tool_calls.insert(call_id.to_owned());
            }
            if operation == "append" {
                explicit.insert(id.to_owned(), items.len());
                items.push(json!({
                    "id": id,
                    "sequence": sequence,
                    "revision": revision,
                    "timestamp": timestamp,
                    "role": role,
                    "type": entry_type,
                    "final": final_entry,
                    "body": body
                }));
            } else if let Some(index) = explicit.get(id).copied() {
                let sequence = items[index]["sequence"].clone();
                let original_timestamp = items[index]["timestamp"].clone();
                items[index] = json!({
                    "id": id,
                    "sequence": sequence,
                    "revision": revision,
                    "timestamp": original_timestamp,
                    "role": role,
                    "type": entry_type,
                    "final": operation == "finalize" || final_entry,
                    "body": body
                });
            }
            continue;
        }
        if claim.subject == owner && claim.kind == "runtime.observed" {
            if !applies_to_incarnation(fields) {
                continue;
            }
            let Some(observed) = fields.get("status").and_then(Value::as_str) else {
                continue;
            };
            let status = match observed {
                "pending" | "starting" => "queued",
                "running" | "ready" | "working" => "running",
                "idle" | "blocked" => "waiting",
                "stopped" | "exited" | "absent" => "completed",
                "failed" => "failed",
                "cancelled" => "cancelled",
                _ => continue,
            };
            items.push(json!({
                "id": entry_id(&claim, "runtime-status"), "sequence": base_sequence,
                "revision": 1, "timestamp": timestamp, "role": "system", "type": "status",
                "final": true, "body": { "status": status, "detail": observed }
            }));
            continue;
        }
        if claim.subject == owner && claim.kind == "harness.observed" {
            if !applies_to_incarnation(fields) {
                continue;
            }
            let observed = fields
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("indeterminate");
            let status = match observed {
                "starting" => "queued",
                "ready" | "working" => "running",
                "idle" | "blocked" => "waiting",
                "ended" => "completed",
                _ => "failed",
            };
            items.push(json!({
                "id": entry_id(&claim, "harness-status"), "sequence": base_sequence,
                "revision": 1, "timestamp": timestamp, "role": "system", "type": "status",
                "final": true, "body": { "status": status, "detail": observed }
            }));
            continue;
        }
        if claim.subject == owner && claim.kind == "harness.diagnostic" {
            // Protocol initialization is informational component evidence, not an error.
            if fields.get("code").and_then(Value::as_str) == Some("claude-channel-initialized") {
                continue;
            }
            if !applies_to_incarnation(fields) {
                continue;
            }
            items.push(json!({
                "id": entry_id(&claim, "diagnostic"), "sequence": base_sequence,
                "revision": 1, "timestamp": timestamp, "role": "system", "type": "error",
                "final": true, "body": {
                    "code": fields.get("code").and_then(Value::as_str).unwrap_or("harness-diagnostic"),
                    "message": fields.get("reason").and_then(Value::as_str).unwrap_or("the harness reported a diagnostic"),
                    "retryable": fields.get("severity").and_then(Value::as_str) == Some("warning"),
                    "details": { "severity": fields.get("severity"), "claim_id": claim.id }
                }
            }));
            continue;
        }
        if claim.subject == owner && claim.kind == "harness.usage" {
            if fields.get("semantics").and_then(Value::as_str) == Some("response_rollup") {
                continue;
            }
            if !applies_to_incarnation(fields) {
                continue;
            }
            let mut body = fields.as_object().cloned().unwrap_or_default();
            body.remove("incarnation_id");
            let body = normalized_timeline_usage_body(
                Value::Object(body),
                &attribution,
                fields.get("driver").and_then(Value::as_str),
            );
            items.push(json!({
                "id": entry_id(&claim, "usage"), "sequence": base_sequence,
                "revision": 1, "timestamp": timestamp, "role": "system", "type": "usage",
                "final": true, "body": body
            }));
            continue;
        }
        if claim.kind == "message.sent" {
            // Only session_messages put a message here.
            let from = fields.get("from").and_then(Value::as_str);
            let role = if from == Some(owner) {
                "assistant"
            } else {
                "user"
            };
            items.push(json!({
                "id": entry_id(&claim, "message"), "sequence": base_sequence,
                "revision": 1, "timestamp": timestamp, "role": role, "type": "message",
                "final": true, "body": session_message_body(state, &claim, session.conversation_message_participants)
            }));
            items.push(json!({
                "id": entry_id(&claim, "content"), "sequence": base_sequence + 1,
                "revision": 1, "timestamp": timestamp, "role": role, "type": "content",
                "final": true, "body": session_message_content(session, &session_id, &claim)
            }));
        }
    }
    items.extend(transcript_notice_entry);
    if has_older_timeline || prefix_unavailable {
        let mut notice_sequence = items
            .iter()
            .filter_map(|item| item["sequence"].as_u64())
            .max()
            .unwrap_or(0);
        // The replicated snapshot clock can precede newer local observations.
        // A projection notice belongs after the materialized entries in both orders.
        let notice_timestamp = items
            .iter()
            .filter_map(|item| item["timestamp"].as_str())
            .max_by_key(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
            .unwrap_or(&snapshot.created_at);
        let mut notice = |code: &str, message: &str, details: Value| {
            notice_sequence = notice_sequence.saturating_add(1);
            json!({
                "id": format!("timeline-entry/{session_leaf}/{code}"),
                "sequence": notice_sequence,
                "revision": 1, "timestamp": notice_timestamp,
                "role": "system", "type": "error", "final": true,
                "body": {"code":code, "message":message, "retryable":false, "details":details}
            })
        };
        let query_notice = has_older_timeline.then(|| {
            notice(
                "timeline-query-limited",
                "Only the newest 4096 stored transcript operations are available in this view. Older operations and entries updated across that boundary are not included; page cursors cover only this view.",
                json!({"operation_limit":4096, "omitted_updated_entries":omitted_updates.len()}),
            )
        });
        let prefix_notice = prefix_unavailable.then(|| {
            notice(
                "timeline-history-incomplete",
                "Earlier transcript history is unavailable: the physical retained prefix has no covering truncation interval.",
                json!({"retained_history_incomplete":true, "full_resync":false}),
            )
        });
        items.extend([query_notice, prefix_notice].into_iter().flatten());
    }
    items.sort_by(timeline_order);
    if let Some(extent) = extent {
        extent(&items);
    }
    let header = conversation_page_header(state, session, &session_id, &items, false);
    // A conversation opens at its newest bounded window. The cursor walks toward older
    // windows, while each individual page remains chronological for straightforward rendering.
    items.reverse();
    let mut page = client_page(
        state,
        snapshot,
        &format!("timeline/{session_id}"),
        items,
        query,
    )?;
    page.items.reverse();
    let mut page_value = json!({
        "kind": "timeline-page",
        "session_id": session_id,
        "items": page.items,
        "page": page.page
    });
    if let Some(header) = header {
        page_value["header"] = header;
    }
    Ok(Json(page_value))
}

/// Resolve only where a conversation is read, without making a child a managed session.
pub fn routing_session_owner_at(
    store: &Store, index: u64, session_id: &str,
) -> anyhow::Result<Option<super::ManagedSessionOwner>> {
    let child = crate::subagent_sessions::child_parts(session_id);
    let routing_id = child.as_ref()
        .map_or(session_id, |parts| parts.routing_parent_session_id.as_str());
    super::managed_session_owner_at(store, index, routing_id)
}

pub fn conversation_session_id(state: &AppState, id: &str) -> Result<String, ApiError> {
    if id.starts_with("agent/") {
        let frontier = state.store.index().map_err(ApiError::internal)?;
        let owner = state.store.conversation_owner_at(id, frontier).map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found(format!("agent `{id}` does not exist")))?;
        let incarnation = owner.incarnation.as_deref().or(owner.runtime.as_deref())
            .ok_or_else(|| validation("the agent has no current session"))?;
        return Ok(client_session_id(id, incarnation));
    }
    if id.starts_with("message/") {
        let claim = state
            .store
            .claims_for(id, Some("message.sent"))
            .map_err(ApiError::internal)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found(format!("message `{id}` does not exist")))?;
        let session_id = claim
            .body
            .pointer("/fields/session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| validation("the message has no session peer"))?;
        return Ok(session_id.to_owned());
    }
    Ok(client_detail_id("session", id))
}

#[derive(Default, Deserialize)]
pub struct EventsQuery {
    pub after: Option<String>,
    pub limit: Option<usize>,
    pub wait_ms: Option<u64>,
}

fn decode_event_cursor(node: &str, cursor: Option<&str>) -> Result<EventCursor, ApiError> {
    let Some(cursor) = cursor else {
        return Ok(EventCursor {
            claim: 0,
            local: None,
        });
    };
    let mut parts = cursor.split('/');
    if parts.next() != Some("event-cursor") || parts.next() != Some(node) {
        return Err(ApiError {
            status: StatusCode::GONE,
            code: "cursor-gap".into(),
            message: "the event cursor belongs to another host or retention epoch".into(),
            details: Box::new(serde_json::Map::from_iter([(
                "full_resync".into(),
                Value::Bool(true),
            )])),
        });
    }
    parts
        .next()
        .and_then(|value| match value.split_once('.') {
            Some((claim, local)) => Some(EventCursor {
                claim: claim.parse().ok()?,
                local: Some(local.parse().ok()?),
            }),
            None => Some(EventCursor {
                claim: value.parse().ok()?,
                local: None,
            }),
        })
        .filter(|_| parts.next().is_none())
        .ok_or_else(|| validation("the event cursor is malformed"))
}

fn event_resume_floor(oldest: u64) -> u64 {
    oldest.saturating_sub(1)
}

pub fn validate_event_cursor(
    node: &str,
    cursor_was_supplied: bool,
    after: u64,
    oldest: u64,
    newest: u64,
) -> Result<(), ApiError> {
    let floor = event_resume_floor(oldest);
    if cursor_was_supplied && (after < floor || after > newest) {
        return Err(ApiError {
            status: StatusCode::GONE,
            code: "cursor-gap".into(),
            message: "the event cursor is outside the retained event range".into(),
            details: Box::new(serde_json::Map::from_iter([
                ("full_resync".into(), Value::Bool(true)),
                (
                    "oldest_cursor".into(),
                    Value::String(format!("event-cursor/{node}/{floor}")),
                ),
                (
                    "newest_cursor".into(),
                    Value::String(format!("event-cursor/{node}/{newest}")),
                ),
            ])),
        });
    }
    Ok(())
}

pub fn client_session_id(owner: &str, incarnation: &str) -> String {
    let digest = hex::encode(Sha256::digest(format!("{owner}:{incarnation}").as_bytes()));
    format!("session/{}", &digest[..24])
}

/// What a page of events needs from the store to attribute its subjects, read once per page
/// instead of once per event.
#[derive(Default)]
pub struct EventPageLookups {
    missions: BTreeMap<String, String>,
    custom_sources: BTreeSet<String>,
}

impl EventPageLookups {
    pub fn read<'a>(state: &AppState, records: impl IntoIterator<Item = &'a EventRecord>) -> Self {
        let (mut runs, mut customs) = (BTreeSet::new(), BTreeSet::new());
        for record in records {
            if record.subject.starts_with("custom/") {
                customs.insert(record.subject.clone());
            } else if record.subject.starts_with("mission-run/") {
                runs.insert(record.subject.clone());
            }
        }
        Self {
            missions: state
                .store
                .run_missions(&runs.into_iter().collect::<Vec<_>>())
                .unwrap_or_default(),
            custom_sources: state
                .store
                .registered_custom_subjects(&customs.into_iter().collect::<Vec<_>>())
                .unwrap_or_default(),
        }
    }
}

pub fn safe_event_projection_with(
    state: &AppState,
    record: &EventRecord,
    lookups: &EventPageLookups,
) -> (String, Vec<String>, Value) {
    if record.subject.starts_with("custom/") && lookups.custom_sources.contains(&record.subject) {
        return (
            "attention.changed".into(),
            vec![record.subject.clone()],
            json!({"reason":"registered-custom-source-changed"}),
        );
    }
    let fields = record.body.get("fields").unwrap_or(&record.body);
    if record.kind == "harness.timeline" {
        let resource_ids = fields
            .get("incarnation_id")
            .and_then(Value::as_str)
            .map(|incarnation| vec![client_session_id(&record.subject, incarnation)])
            .unwrap_or_default();
        return (
            "upsert".into(),
            resource_ids,
            json!({ "reason": "session-timeline-invalidated" }),
        );
    }
    if record.kind.starts_with("custom.client.pairing-") {
        return (
            "capabilities.changed".into(),
            Vec::new(),
            json!({ "reason": "authenticated-client-capabilities-changed" }),
        );
    }
    if record.kind.starts_with("custom.client.terminal-") {
        let resource_ids = fields
            .get("terminal_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                state
                    .store
                    .claims_for(&record.subject, Some("custom.client.terminal-attached"))
                    .ok()?
                    .into_iter()
                    .next()?
                    .body
                    .pointer("/fields/terminal_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .map(|terminal| vec![terminal.to_owned()])
            .unwrap_or_default();
        return (
            "upsert".into(),
            resource_ids,
            json!({ "reason": "terminal-viewer-lifecycle-changed" }),
        );
    }
    let mut resource_ids = Vec::new();
    if record.subject.starts_with("message/")
        || record.subject.starts_with("attention/")
        || record.subject.starts_with("agent/")
        || record.subject.starts_with("observer/")
        || record.subject.starts_with("subscription/")
        || record.subject.starts_with("step-run/")
        || record.subject.starts_with("mission/")
    {
        resource_ids.push(record.subject.clone());
    } else if record.subject.starts_with("mission-run/")
        && let Some(mission) = lookups.missions.get(&record.subject)
    {
        resource_ids.push(mission.clone());
    } else if record.subject.starts_with("planning-session/") {
        resource_ids.push(format!(
            "launch/{}",
            record.subject.trim_start_matches("planning-session/")
        ));
    }
    if record.kind == "message.sent"
        && let Some(session_id) = fields.get("session_id").and_then(Value::as_str)
    {
        resource_ids.push(session_id.to_owned());
    }
    let event_type = if record.kind == "runtime.observed"
        && fields.get("status").and_then(Value::as_str) == Some("running")
        && fields.get("terminal").and_then(Value::as_bool) == Some(true)
    {
        "terminal.available"
    } else {
        "upsert"
    };
    (
        event_type.into(),
        resource_ids,
        json!({
            "reason": "client-projection-invalidated",
            "change": record.kind,
            "subject": record.subject,
            "state": fields.get("state").or_else(|| fields.get("status")).cloned()
        }),
    )
}

pub async fn events(
    State(state): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Value>, ApiError> {
    require_scope(&session, "read.projections")?;
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    // Registration precedes both the floor/frontier and empty-page reads.
    let mut changed = state.event_notify.subscribe();
    let after = decode_event_cursor(&state.node, query.after.as_deref())?;
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(query.wait_ms.unwrap_or(0).min(30_000));
    // Subscribe before the first store read. An event between reading an empty
    // page and subscribing must wake this long poll, not wait for another event.
    let (records, oldest, newest) = loop {
        let (records, oldest, newest) =
            super::read_deadline::query(&state.store, "/v1/client/events", || {
                state
                    .store
                    .read_snapshot(|_| {
                        let (oldest, newest) = state.store.event_bounds()?;
                        if let Err(error) = validate_event_cursor(
                            &state.node,
                            query.after.is_some(),
                            after.claim,
                            oldest,
                            newest,
                        ) {
                            return Ok(Err(error));
                        }
                        let records = if query.after.is_some() {
                            feed_events_after(&state.store, after, limit.saturating_add(1))?
                        } else {
                            feed_events_tail(&state.store, limit)?
                        };
                        Ok(Ok((records, oldest, newest)))
                    })
                    .map_err(ApiError::internal)?
            })?;
        if !records.is_empty() || tokio::time::Instant::now() >= deadline {
            break (records, oldest, newest);
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if !matches!(
            tokio::time::timeout(remaining, changed.changed()).await,
            Ok(Ok(()))
        ) {
            // Read the current floor again at timeout; migration may not emit a normal event.
            continue;
        }
    };
    let has_more = query.after.is_some() && records.len() > limit;
    let records = records.into_iter().take(limit).collect::<Vec<_>>();
    let mut resume = records
        .last()
        .map(|(record, local)| EventCursor {
            claim: record.store_index,
            local: *local,
        })
        .unwrap_or(after);
    if query.after.is_none() && resume.claim < newest {
        // A bounded replacement at resync may show the latest retained activity, but its next
        // request must start at the observed current frontier, not below the new upgrade floor.
        resume = EventCursor {
            claim: newest,
            local: None,
        };
    }
    // The projection enrichment reads the store (mission attribution, snapshot ids), so it
    // takes one admitted query scope; the wait above held no reader.
    let items = super::read_deadline::query(&state.store, "/v1/client/events", || {
        // Every event is named by its own cut; read all of the page's cuts in one statement.
        let cuts = records
            .iter()
            .map(|(record, _)| record.store_index)
            .collect::<Vec<_>>();
        let times = state.store.projection_times_at(&cuts).unwrap_or_default();
        let lookups = EventPageLookups::read(&state, records.iter().map(|(record, _)| record));
        Ok(records
            .into_iter()
            .map(|(record, local)| {
                let position = EventCursor {
                    claim: record.store_index,
                    local,
                };
                let previous = match local {
                    Some(local) => EventCursor {
                        claim: record.store_index,
                        local: Some(local.saturating_sub(1)),
                    },
                    None => EventCursor {
                        claim: record.store_index.saturating_sub(1),
                        local: None,
                    },
                };
                let event_snapshot = client_snapshot_with_time(
                    &state,
                    record.store_index,
                    times.get(&record.store_index).copied().unwrap_or_default(),
                );
                let (event_type, resource_ids, body) = safe_event_projection_with(&state, &record, &lookups);
                json!({
                    "id": format!("projection-event/{}/{}", state.node, position.label()),
                    "epoch": state.node,
                    "sequence": record.store_index,
                    "previous_cursor": previous.encode(&state.node),
                    "next_cursor": position.encode(&state.node),
                    "timestamp": event_snapshot.created_at,
                    "type": event_type,
                    "resource_ids": resource_ids,
                    "snapshot_id": event_snapshot.id,
                    "body": body
                })
            })
            .collect::<Vec<_>>())
    })?;
    Ok(Json(json!({
        "kind": "event-page",
        "oldest_cursor": format!("event-cursor/{}/{}", state.node, event_resume_floor(oldest)),
        "resume_cursor": resume.encode(&state.node),
        "items": items,
        "has_more": has_more
    })))
}

/// A position in this node's event feed. Claim events come in store order. A local
/// observation event follows the claim it was written after, so `local` names the last
/// local observation delivered after `claim`; `None` means none of them yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventCursor {
    claim: u64,
    local: Option<u64>,
}

impl EventCursor {
    fn label(&self) -> String {
        match self.local {
            Some(local) => format!("{}.{local}", self.claim),
            None => self.claim.to_string(),
        }
    }

    fn encode(&self, node: &str) -> String {
        format!("event-cursor/{node}/{}", self.label())
    }
}

type FeedEvent = (EventRecord, Option<u64>);

fn local_feed_event(record: ClaimRecord) -> FeedEvent {
    let local = crate::store::local_observation_position(&record);
    (
        EventRecord {
            store_index: record.store_index,
            kind: record.kind,
            subject: record.subject,
            body: record.body,
        },
        local,
    )
}

fn feed_order(event: &FeedEvent) -> (u64, u64) {
    (event.0.store_index, event.1.unwrap_or(0))
}

/// Claim events and local observation events after `after`, oldest first.
fn feed_events_after(
    store: &Store,
    after: EventCursor,
    limit: usize,
) -> anyhow::Result<Vec<FeedEvent>> {
    let local_after = match after.local {
        Some(local) => local,
        None => store.local_observation_floor_after_claim(after.claim)?,
    };
    let mut events = store
        .events_after_bounded(after.claim, limit)?
        .into_iter()
        .map(|record| (record, None))
        .collect::<Vec<_>>();
    events.extend(
        store
            .local_observations_after(local_after, limit)?
            .into_iter()
            .map(local_feed_event),
    );
    events.sort_by_key(feed_order);
    events.truncate(limit);
    Ok(events)
}

/// The newest `limit` claim and local observation events, oldest first.
fn feed_events_tail(store: &Store, limit: usize) -> anyhow::Result<Vec<FeedEvent>> {
    let mut events = store
        .events_tail_bounded(limit)?
        .into_iter()
        .map(|record| (record, None))
        .collect::<Vec<_>>();
    events.extend(
        store
            .local_observations_tail(limit)?
            .into_iter()
            .map(local_feed_event),
    );
    events.sort_by_key(feed_order);
    let excess = events.len().saturating_sub(limit);
    events.drain(..excess);
    Ok(events)
}

pub fn terminal_subject(id: &str) -> String {
    id.strip_prefix("terminal/").unwrap_or(id).to_owned()
}

pub fn terminal_live_session(
    state: &AppState,
    subject: &str,
    expected_incarnation: Option<&str>,
) -> Result<LiveSession, ApiError> {
    live_session(state, subject, expected_incarnation).map_err(|error| {
        if error.code == "stale-incarnation" {
            stale(error.message)
        } else {
            error
        }
    })
}

/// How long a viewer waits for a terminal's first screen before giving up.
pub const TERMINAL_FIRST_SCREEN_TIMEOUT: Duration = Duration::from_secs(5);
pub fn terminal_unavailable(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "terminal-unavailable".into(),
        message: message.into(),
        details: Box::default(),
    }
}

pub fn terminal_view_error(end: terminal_view::ViewEnd) -> ApiError {
    match end {
        terminal_view::ViewEnd::Unavailable(message) => terminal_unavailable(message),
        terminal_view::ViewEnd::Idle => terminal_unavailable("the terminal viewer became idle"),
        terminal_view::ViewEnd::Exited => ApiError {
            status: StatusCode::GONE,
            code: "terminal-ended".into(),
            message: "the terminal session ended".into(),
            details: Box::default(),
        },
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fence {
    pub snapshot_id: String,
    #[serde(default)]
    pub subject_revisions: BTreeMap<String, String>,
    pub mission_generation: Option<String>,
    pub step_definition: Option<String>,
    pub attempt: Option<u32>,
    pub readiness_epoch: Option<u64>,
    pub runtime_incarnation: Option<String>,
    pub runtime_desired_revision: Option<String>,
    pub terminal_sequence: Option<u64>,
    pub preview_token: Option<String>,
    pub private_notes: Option<crate::private_notes::NotesFence>,
}

pub fn action_scope(action: &str) -> Option<&'static str> {
    if action == "harness.answer" {
        return Some("terminal.control");
    }
    if action == "private-notes.write" { return Some("control.work"); }
    if matches!(
        action,
        "agent.create"
            | "agent.stop"
            | "agent.retire"
            | "agent.start"
            | "agent.suspend"
            | "agent.resume"
    ) {
        return Some("control.runtimes");
    }
    if matches!(action, "work.done" | "custom.reply" | "prompt.respond") {
        return Some("control.attention");
    }
    Some(match action.split_once('.')?.0 {
        "attention" => "control.attention",
        "review" => "control.attention",
        "message" => "control.messages",
        "launch" => "control.launches",
        "mission" => "control.missions",
        "session" => "control.missions",
        "work" => "control.work",
        "agent" => "control.work",
        "lane" => "control.work",
        "runtime" => "control.runtimes",
        "terminal" => {
            if matches!(action, "terminal.attach" | "terminal.detach") {
                "terminal.read"
            } else {
                "terminal.control"
            }
        }
        "pairing" => "control.pairing",
        "arrangement" => "control.arrangements",
        _ => return None,
    })
}

pub fn require_creation_actor(session: &ClientSession) -> Result<&str, ApiError> {
    if !acting_party(session) {
        return Err(forbidden(
            "creation requires the session's concrete person or local agent",
        ));
    }
    Ok(&session.authority_actor)
}


pub fn glass_person(session: &ClientSession, write: bool) -> Result<String, ApiError> {
    require_scope(
        session,
        if write {
            "control.glasses"
        } else {
            "read.glasses"
        },
    )?;
    let person = &session.authority_actor;
    if !person.starts_with("person/") || person.matches('/').count() != 1 {
        return Err(forbidden("glasses require the session's concrete person"));
    }
    Ok(person.clone())
}

#[cfg(any(test, feature = "test-support"))]
pub static RUNTIME_DETAIL_SUBJECT_COUNTS: std::sync::Mutex<BTreeMap<String, usize>> =
    std::sync::Mutex::new(BTreeMap::new());

/// First-page builds for initial OPEN and reactive changes, counted by the work-budget tests.
#[cfg(any(test, feature = "test-support"))]
pub fn timeline_rebuilds() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    static REBUILDS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        std::sync::OnceLock::new();
    REBUILDS.get_or_init(Default::default)
}
