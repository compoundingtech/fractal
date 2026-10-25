//! Durable public names over immutable claim/storage subjects.
//!
//! Names are never reused. Partition collisions are reduced by claim ID, not arrival time:
//! the smallest claim owns the target, while every losing fact remains in the claim log and
//! fences its identity. The projection can always be reconstructed from durable claims.
use super::*;
use serde::Deserialize;

pub const AGENT_RENAMED: &str = "agent.renamed";
pub const MISSION_RENAMED: &str = "mission.renamed";
pub const KINDS: &[&str] = &[AGENT_RENAMED, MISSION_RENAMED];

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityKind { Agent, Mission }

impl IdentityKind {
    pub fn prefix(self) -> &'static str { match self { Self::Agent => "agent/", Self::Mission => "mission/" } }
    pub fn claim_kind(self) -> &'static str { match self { Self::Agent => AGENT_RENAMED, Self::Mission => MISSION_RENAMED } }
    fn label(self) -> &'static str { match self { Self::Agent => "agent", Self::Mission => "mission" } }
    fn of(id: &str) -> Option<Self> {
        if id.starts_with("agent/") { Some(Self::Agent) } else if id.starts_with("mission/") { Some(Self::Mission) } else { None }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedIdentity {
    pub kind: IdentityKind,
    pub identity: String,
    pub canonical_id: String,
    pub naming_revision: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RenameRequest {
    pub from: String,
    pub to: String,
    pub actor: String,
    pub reason: String,
    #[serde(default)]
    pub expected_naming_revision: Option<String>,
    #[serde(default)]
    pub expected_declaration: Option<String>,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RenameReplication { pub state: String, pub frontier: u64 }
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RenameReceipt {
    pub operation_id: String,
    pub idempotency_key: String,
    pub applied: String,
    pub kind: IdentityKind,
    pub identity: String,
    pub from: String,
    pub to: String,
    pub canonical_id: String,
    pub naming_revision: String,
    pub claim_id: String,
    pub preserved_incarnation: Option<String>,
    pub replication: RenameReplication,
}

pub trait IdentityStore {
    fn resolve_identity(&self, kind: IdentityKind, id: &str) -> Result<ResolvedIdentity, St3Error>;
    fn rename_identity(&self, kind: IdentityKind, request: &RenameRequest, peers: &[String]) -> Result<RenameReceipt, St3Error>;
}

pub fn unavailable(id: &str, reason: &str) -> St3Error {
    St3Error::new("lifecycle-unavailable", format!("identity `{id}` is unavailable: {reason}"))
        .with_detail("subject", id).with_detail("reason", reason).with_detail("applied", "none")
}
fn cross_kind(id: &str, kind: IdentityKind) -> St3Error {
    St3Error::new("identity-cross-kind", format!("`{id}` is not a {} identity", kind.label()))
        .with_detail("subject", id).with_detail("source_kind", kind.label())
}
fn validate_name(kind: IdentityKind, id: &str) -> Result<(), St3Error> {
    if IdentityKind::of(id) != Some(kind) { return Err(cross_kind(id, kind)); }
    fractal_schema::registry().validate_subject(id).map_err(|e| St3Error::new(e.code, e.message))?;
    Ok(())
}

/// Resolve in the caller's snapshot; no nested reader or writer acquisition.
pub fn resolve_identity_conn(connection: &Connection, kind: IdentityKind, id: &str) -> Result<ResolvedIdentity, St3Error> {
    validate_name(kind, id)?;
    let row: Option<(String, String, Option<String>, Option<String>)> = connection.query_row(
        "SELECT h.identity,h.canonical_id,h.naming_revision,h.unavailable FROM identity_names n JOIN identity_heads h ON h.identity=n.identity WHERE n.name=?1 AND h.kind=?2",
        params![id,kind.label()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional().map_err(internal)?;
    let (identity, canonical_id, naming_revision, blocked) = if let Some(row) = row { row } else {
        if let Some(reason) = pending(connection, id)? { return Err(unavailable(id, &reason)); }
        if declaration_token(connection, kind, id)?.is_none() {
            return Err(St3Error::new("identity-unknown", format!("identity `{id}` is unknown")).with_detail("subject", id));
        }
        (id.into(), id.into(), None, None)
    };
    if let Some(reason) = blocked { return Err(unavailable(&identity, &reason)); }
    if let Some(reason) = pending(connection, &identity)? { return Err(unavailable(&identity, &reason)); }
    Ok(ResolvedIdentity { kind, identity, canonical_id, naming_revision })
}

/// Normalize references without making previously permissible undeclared references invalid.
/// A known alias with missing/conflicting lifecycle evidence is never allowed through.
pub fn resolve_reference_conn(connection: &Connection, id: &str) -> Result<String, St3Error> {
    let Some(_) = IdentityKind::of(id) else { return Ok(id.into()); };
    // Reference readers also encounter historical storage keys that predate today's
    // declaration grammar. Admission validates new names; reading an old key must not.
    let (identity, blocked, pending): (String, Option<String>, Option<String>) = connection.query_row(
        "SELECT COALESCE(n.identity,?1),h.unavailable,
            (SELECT CASE WHEN COUNT(*)=0 THEN NULL WHEN MAX(state='invalid')=1
                THEN 'conflict' ELSE 'pending-replication' END FROM replica_records
             WHERE subject_hint=COALESCE(n.identity,?1) AND state IN ('unknown','invalid')
             AND (kind_hint IN ('agent.renamed','mission.renamed','agent.retired','intent.desired','mission.published') OR kind_hint IS NULL))
         FROM (SELECT 1) LEFT JOIN identity_names n ON n.name=?1
         LEFT JOIN identity_heads h ON h.identity=n.identity",
        [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(internal)?;
    if let Some(reason) = blocked.or(pending) { return Err(unavailable(&identity, &reason)); }
    Ok(identity)
}

pub fn normalize_claim_input<'a>(connection: &Connection, input: &'a ClaimInput) -> Result<std::borrow::Cow<'a, ClaimInput>, St3Error> {
    let mut normalized = std::borrow::Cow::Borrowed(input);
    let identity: Option<String> = connection.query_row("SELECT identity FROM identity_names WHERE name=?1 AND name<>identity", [&input.subject], |row|row.get(0)).optional().map_err(internal)?;
    if let Some(identity) = identity { normalized.to_mut().subject = identity; }
    if let Some(actor) = input.actor.as_deref() {
        let identity: Option<String> = connection.query_row("SELECT identity FROM identity_names WHERE name=?1 AND name<>identity", [actor], |row|row.get(0)).optional().map_err(internal)?;
        if let Some(identity) = identity { normalized.to_mut().actor = Some(identity); }
    }
    // Naming facts deliberately carry the previous public canonical name, not a rewritten
    // storage spelling. Their embedded request is also the exact idempotency input.
    if !KINDS.contains(&input.kind.as_str()) {
        for field in ["from", "to", "assignee", "recipient", "report_to", "lease_owner"] {
            if let Some(id) = input.fields.get(field).and_then(Value::as_str) {
                let identity: Option<String> = connection.query_row("SELECT identity FROM identity_names WHERE name=?1 AND name<>identity", [id], |row|row.get(0)).optional().map_err(internal)?;
                if let Some(identity) = identity { normalized.to_mut().fields.insert(field.into(), json!(identity)); }
            }
        }
        // This is local admission only: bind placement targets before operation hashing
        // and signing. Projectors must replay the signed identity without alias lookup.
        if input.kind == "arrangement.edited"
            && let Some(operations) = input.fields.get("operations").and_then(Value::as_array) {
            for (index, operation) in operations.iter().enumerate() {
                if operation["op"].as_str() != Some("subject.place") { continue; }
                let Some(subject) = operation["subject"].as_str() else { continue; };
                let identity = resolve_reference_conn(connection, subject)?;
                if identity != subject {
                    normalized.to_mut().fields.get_mut("operations").expect("operations array")[index]["subject"] = json!(identity);
                }
            }
        }
    }
    Ok(normalized)
}

/// Admission-derived declaration/head evidence is not part of the caller's idempotency input.
/// Hash the exact request, so two identical concurrent requests still identify one operation.
pub fn claim_operation(input: &ClaimInput) -> Result<Option<(String,String)>, St3Error> {
    if !KINDS.contains(&input.kind.as_str()) { return smallclaims::store::claim_operation(input); }
    let key = input.idempotency_key.as_deref().ok_or_else(|| St3Error::new("invalid-rename", "identity rename requires an idempotency key"))?;
    if key.is_empty() || key.len() > 512 || key.chars().any(char::is_control) {
        return Err(St3Error::new("invalid-idempotency-key", "an idempotency key must contain 1 through 512 non-control characters"));
    }
    let request = input.fields.get("request").ok_or_else(|| St3Error::new("invalid-rename", "naming claim is missing its request"))?;
    if request["idempotency_key"].as_str() != Some(key) {
        return Err(St3Error::new("invalid-rename", "naming request and operation keys disagree"));
    }
    Ok(Some((operation_id_for_key(key),request_digest(&input.kind, request)?)))
}

fn request_digest(kind: &str, request: &Value) -> Result<String, St3Error> {
    canonical_hash(&("st3.identity-rename-request.v1",kind,canonical_json_value(request))).map_err(internal)
}

/// Check immutable request/effect binding before a replicated fact enters the projection.
/// Missing declaration/predecessor evidence is intentionally left to the fail-closed reducer.
pub fn validate_replicated(claim: &ClaimRecord) -> Result<(), St3Error> {
    if !KINDS.contains(&claim.kind.as_str()) { return Ok(()); }
    let invalid = || St3Error::new("invalid-replicated-claim", "naming claim request, operation or owner evidence disagrees");
    let fields = &claim.body["fields"];
    let request_value = &fields["request"];
    let request: RenameRequest = RenameRequest::deserialize(request_value).map_err(|_| invalid())?;
    let kind = if claim.kind == AGENT_RENAMED { IdentityKind::Agent } else { IdentityKind::Mission };
    if IdentityKind::of(&claim.subject) != Some(kind)
        || IdentityKind::of(&request.from) != Some(kind)
        || IdentityKind::of(&request.to) != Some(kind)
        || request.reason.trim().is_empty()
        || fields["to"].as_str() != Some(request.to.as_str())
        || fields["reason"].as_str() != Some(request.reason.as_str())
        || fields["owner_host"].as_str() != Some(claim.origin.as_str())
        || claim.actor.is_none()
        || (!request.actor.starts_with("agent/") && claim.actor.as_deref() != Some(request.actor.as_str()))
        || request.expected_declaration.as_deref().is_some_and(|expected| fields["declaration"].as_str() != Some(expected))
        || request.expected_naming_revision.as_deref().is_some_and(|expected| fields["previous_revision"].as_str() != Some(expected))
    { return Err(invalid()); }
    let Some((operation,digest)) = operation_parts(&claim.body) else { return Err(invalid()); };
    if operation != operation_id_for_key(&request.idempotency_key) || digest != request_digest(&claim.kind, request_value)? {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn check_receipt(connection: &Connection, claim: &ClaimRecord) -> Result<(), St3Error> {
    if !KINDS.contains(&claim.kind.as_str()) { return Ok(()); }
    let repaired: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND state='repaired')", [&claim.id], |row|row.get(0)).map_err(internal)?;
    if repaired { return Err(unavailable(&claim.subject, "conflict")); }
    let kind = if claim.kind == AGENT_RENAMED { IdentityKind::Agent } else { IdentityKind::Mission };
    resolve_identity_conn(connection, kind, &claim.subject)?;
    Ok(())
}

/// Normalize only typed KDL reference arguments, never command/content/environment strings.
fn normalize_node<'a>(connection: &Connection, node: &'a Value) -> Result<std::borrow::Cow<'a, Value>, St3Error> {
    let name = node["name"].as_str().unwrap_or("");
    let reference = matches!(name, "agent" | "mission" | "from" | "to" | "assignee" | "available-to" | "report-to" | "under" | "recipient");
    let mut normalized = std::borrow::Cow::Borrowed(node);
    if reference && let Some(arguments) = node.get("arguments").and_then(Value::as_array) {
        for (index, argument) in arguments.iter().enumerate() {
            let Some(value) = argument.as_str() else { continue; };
            let prefix = match name { "agent" => Some("agent/"), "mission" => Some("mission/"), _ => None };
            let full = prefix.filter(|prefix| !value.starts_with(prefix)).map(|prefix| format!("{prefix}{value}"));
            let identity: Option<String> = connection.query_row("SELECT identity FROM identity_names WHERE name=?1 AND name<>identity",
                [full.as_deref().unwrap_or(value)], |row| row.get(0)).optional().map_err(internal)?;
            if let Some(identity) = identity {
                let identity = if let Some(prefix) = prefix.filter(|_| full.is_some()) { identity.strip_prefix(prefix).unwrap_or(&identity) } else { &identity };
                normalized.to_mut()["arguments"][index] = json!(identity);
            }
        }
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for (index, child) in children.iter().enumerate() {
            if let std::borrow::Cow::Owned(child) = normalize_node(connection, child)? {
                normalized.to_mut()["children"][index] = child;
            }
        }
    }
    Ok(normalized)
}

pub fn normalize_desired<'a>(connection: &Connection, desired: &'a DesiredSubject) -> Result<std::borrow::Cow<'a, DesiredSubject>, St3Error> {
    let identity: Option<String> = connection.query_row("SELECT identity FROM identity_names WHERE name=?1 AND name<>identity", [&desired.subject], |row| row.get(0)).optional().map_err(internal)?;
    let node = normalize_node(connection, &desired.desired)?;
    if identity.is_none() && matches!(&node, std::borrow::Cow::Borrowed(_)) {
        return Ok(std::borrow::Cow::Borrowed(desired));
    }
    let mut normalized = DesiredSubject {
        accountable: desired.accountable.clone(), subject: identity.unwrap_or_else(|| desired.subject.clone()),
        kind: desired.kind.clone(), desired: node.into_owned(), member: desired.member.clone(),
        owner_run: desired.owner_run.clone(), owner_generation: desired.owner_generation.clone(), owner_step: desired.owner_step.clone(),
    };
    if normalized.subject != desired.subject && let Some(member) = normalized.member.as_mut() {
        // Runtime resource identity is not a public name and must not be regenerated by parsing.
        let previous: Option<String> = connection.query_row("SELECT member FROM desired WHERE subject=?1", [&normalized.subject], |row| row.get(0)).optional().map_err(internal)?.flatten();
        if let Some(previous) = previous {
            let previous: crate::model::MemberSpec = serde_json::from_str(&previous).map_err(internal)?;
            member.runtime_id = previous.runtime_id;
        }
    }
    Ok(std::borrow::Cow::Owned(normalized))
}

pub fn normalize_intent<'a>(connection: &Connection, intent: &'a NormalizedIntent) -> Result<std::borrow::Cow<'a, NormalizedIntent>, St3Error> {
    for subject in intent.subjects.keys().chain(intent.missions.values().map(|mission| &mission.subject)) {
        resolve_reference_conn(connection, subject)?;
    }
    let aliases: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM identity_names WHERE name<>identity)", [], |row| row.get(0)).map_err(internal)?;
    if !aliases { return Ok(std::borrow::Cow::Borrowed(intent)); }
    let mut normalized = intent.clone();
    normalized.subjects.clear();
    for desired in intent.subjects.values() {
        let desired = normalize_desired(connection, desired)?.into_owned();
        if normalized.subjects.insert(desired.subject.clone(), desired).is_some() {
            return Err(St3Error::new("identity-occupied", "declaration contains multiple spellings of one identity"));
        }
    }
    missions::normalize_intent_missions(connection, &mut normalized)?;
    if let Some(declarations) = normalized.normalized.get_mut("declarations").and_then(Value::as_array_mut) {
        for declaration in declarations {
            if let std::borrow::Cow::Owned(node) = normalize_node(connection, declaration)? { *declaration = node; }
        }
    }
    Ok(std::borrow::Cow::Owned(normalized))
}

fn pending(connection: &Connection, identity: &str) -> Result<Option<String>, St3Error> {
    connection.query_row(
        "SELECT CASE WHEN COUNT(*)=0 THEN NULL WHEN MAX(state='invalid')=1 THEN 'conflict' ELSE 'pending-replication' END FROM replica_records WHERE subject_hint=?1 AND state IN ('unknown','invalid') AND (kind_hint IN ('agent.renamed','mission.renamed','agent.retired','intent.desired','mission.published') OR kind_hint IS NULL)",
        [identity], |row| row.get(0),
    ).map_err(internal)
}
fn declaration_token(connection: &Connection, kind: IdentityKind, identity: &str) -> Result<Option<String>, St3Error> {
    match kind {
        IdentityKind::Agent => connection.query_row("SELECT claim_id FROM desired WHERE subject=?1", [identity], |row| row.get(0)).optional().map_err(internal),
        IdentityKind::Mission => connection.query_row("SELECT claim_id FROM mission_definitions WHERE mission_id=?1", [identity.trim_start_matches("mission/")], |row| row.get(0)).optional().map_err(internal),
    }
}

/// The same persisted registry/membership gate used for terminal retirement.
pub fn check_fleet_registry_conn(connection: &Connection, origin: &str, configured_peers: &[String], digest: &str) -> Result<(), St3Error> {
    let membership = fleet_membership_tx(connection).map_err(internal)?;
    let peers = configured_peers.iter().cloned().chain(membership.incarnations().filter(|member| member.end.is_none()).map(|member| member.name.clone())).collect::<BTreeSet<_>>();
    for peer in peers.into_iter().filter(|peer| peer != origin) {
        let known: Option<String> = connection.query_row("SELECT schema_digest FROM replication_peers WHERE peer=?1", [&peer], |row| row.get(0)).optional().map_err(internal)?.flatten();
        if known.as_deref() != Some(digest) {
            return Err(unavailable(origin, "incompatible-fleet").with_detail("peer", peer));
        }
    }
    Ok(())
}

fn retired_source(connection: &Connection, kind: IdentityKind, identity: &str) -> Result<(), St3Error> {
    let retired = match kind {
        IdentityKind::Agent => retirements::record(connection, identity, None)?.map(|claim| claim.id),
        IdentityKind::Mission => {
            let state: Option<(String,String)> = connection.query_row("SELECT state,claim_id FROM mission_definitions WHERE mission_id=?1", [identity.trim_start_matches("mission/")], |row| Ok((row.get(0)?,row.get(1)?))).optional().map_err(internal)?;
            match state { Some((state,claim)) if state == "retired" => Some(claim), _ => None }
        }
    };
    if let Some(claim) = retired {
        return Err(St3Error::new("identity-retired-source", format!("identity `{identity}` is retired"))
            .with_detail("source", identity).with_detail("retirement_claim", claim));
    }
    if kind == IdentityKind::Mission {
        owned_sets::guard_mission_start(connection, identity).map_err(|error| {
            if error.code == "mission-retired" { St3Error::new("identity-retired-source", error.message).with_detail("source", identity) }
            else { unavailable(identity, "pending-replication") }
        })?;
    }
    Ok(())
}

fn owner_admission(connection: &Connection, kind: IdentityKind, identity: &str, declaration: &str, origin: &str) -> Result<(), St3Error> {
    let claim = claim_by_id_tx(connection, declaration).map_err(internal)?.ok_or_else(|| unavailable(identity, "missing"))?;
    let owner = match kind {
        IdentityKind::Agent => retirements::declaration(connection, identity, declaration)?.member.map(|member| member.host),
        IdentityKind::Mission => Some(claim.origin),
    }.ok_or_else(|| unavailable(identity, "owner-unavailable"))?;
    if owner != origin { return Err(unavailable(identity, "owner-unavailable").with_detail("owner_host", owner)); }
    Ok(())
}

fn occupied(connection: &Connection, kind: IdentityKind, source: &ResolvedIdentity, target: &str) -> Result<(), St3Error> {
    let reserved: Option<String> = connection.query_row("SELECT identity FROM identity_names WHERE name=?1", [target], |row| row.get(0)).optional().map_err(internal)?;
    let reserved = if reserved.is_some() { reserved } else {
        let historical: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind IN ('intent.desired','mission.published'))", [target], |row| row.get(0)).map_err(internal)?;
        historical.then(|| target.to_owned())
    };
    if let Some(identity) = reserved {
        if identity == source.identity {
            return Err(St3Error::new("identity-cycle", "historical names cannot become canonical again")
                .with_detail("from", source.canonical_id.as_str()).with_detail("to", target).with_detail("path", json!([source.canonical_id,target])));
        }
        return Err(St3Error::new("identity-occupied", format!("name `{target}` is already reserved"))
            .with_detail("target", target).with_detail("kind", kind.label()).with_detail("identity", identity));
    }
    if let Some(reason) = pending(connection, target)? { return Err(unavailable(target, &reason)); }
    Ok(())
}

fn receipt(claim: &ClaimRecord, request: &RenameRequest, already: bool) -> RenameReceipt {
    let fields = &claim.body["fields"];
    RenameReceipt {
        operation_id: operation_id_for_key(&request.idempotency_key),
        idempotency_key: request.idempotency_key.clone(),
        applied: if already { "already-applied" } else { "renamed" }.into(),
        kind: if claim.kind == AGENT_RENAMED { IdentityKind::Agent } else { IdentityKind::Mission },
        identity: claim.subject.clone(), from: request.from.clone(), to: request.to.clone(),
        canonical_id: request.to.clone(), naming_revision: claim.id.clone(), claim_id: claim.id.clone(),
        preserved_incarnation: fields["preserved_incarnation"].as_str().map(str::to_owned),
        replication: RenameReplication { state: "local-durable".into(), frontier: claim.store_index },
    }
}

impl IdentityStore for Store {
    fn resolve_identity(&self, kind: IdentityKind, id: &str) -> Result<ResolvedIdentity, St3Error> {
        resolve_identity_conn(&self.readers.get(), kind, id)
    }

    fn rename_identity(&self, kind: IdentityKind, request: &RenameRequest, peers: &[String]) -> Result<RenameReceipt, St3Error> {
        validate_name(kind, &request.from)?;
        validate_name(kind, &request.to)?;
        if request.reason.trim().is_empty() { return Err(St3Error::new("invalid-rename", "identity rename needs a reason")); }
        if request.idempotency_key.is_empty() || request.idempotency_key.len() > 512 || request.idempotency_key.chars().any(char::is_control) {
            return Err(St3Error::new("invalid-idempotency-key", "an idempotency key must contain 1 through 512 non-control characters"));
        }
        let request_value = serde_json::to_value(request).map_err(internal)?;
        let connection = self.readers.get();
        if let Some((_, claim_id, state)) = operation_tx(&connection, &operation_id_for_key(&request.idempotency_key)).map_err(internal)? {
            if state == "conflict" { return Err(St3Error::new("idempotency-conflict", "rename operation has conflicting requests")); }
            let claim = claim_by_id_tx(&connection, &claim_id).map_err(internal)?.ok_or_else(|| unavailable(&request.from, "missing"))?;
            if claim.kind != kind.claim_kind() || claim.body["fields"]["request"] != request_value {
                return Err(St3Error::new("idempotency-mismatch", "idempotency key identifies a different request"));
            }
            check_receipt(&connection, &claim)?;
            return Ok(receipt(&claim, request, true));
        }
        let source = resolve_identity_conn(&connection, kind, &request.from)?;
        let declaration = declaration_token(&connection, kind, &source.identity)?.ok_or_else(|| unavailable(&source.identity, "missing"))?;
        let actor = resolve_reference_conn(&connection, &request.actor)?;
        let incarnation = if kind == IdentityKind::Agent {
            latest_actual(&connection, &source.identity).map_err(internal)?.and_then(|body| body.get("fields").unwrap_or(&body).get("incarnation_id").and_then(Value::as_str).map(str::to_owned))
        } else { None };
        drop(connection);
        let mut input = ClaimInput {
            subject: source.identity.clone(), kind: kind.claim_kind().into(), actor: Some(actor.clone()),
            fields: BTreeMap::from([
                ("from".into(), json!(source.canonical_id)), ("to".into(), json!(request.to)),
                ("reason".into(), json!(request.reason)), ("owner_host".into(), json!(self.origin())),
                ("declaration".into(), json!(declaration)), ("previous_revision".into(), json!(source.naming_revision)),
                ("request".into(), request_value), ("preserved_incarnation".into(), json!(incarnation)),
            ]), evidence: vec![], expected_subject: None, idempotency_key: Some(request.idempotency_key.clone()),
        };
        input.fields.retain(|_, value| !value.is_null());
        self.graph.ensure_principal_key(&actor)?;
        let admission = |connection: &Connection| {
            let current = resolve_identity_conn(connection, kind, &request.from)?;
            retired_source(connection, kind, &current.identity)?;
            if current != source { return Err(unavailable(&source.identity, "stale-revision")); }
            let token = declaration_token(connection, kind, &source.identity)?.ok_or_else(|| unavailable(&source.identity, "missing"))?;
            if token != declaration || request.expected_declaration.as_ref().is_some_and(|expected| expected != &token)
                || request.expected_naming_revision.as_ref().is_some_and(|expected| Some(expected) != current.naming_revision.as_ref()) {
                return Err(unavailable(&source.identity, "stale-revision"));
            }
            occupied(connection, kind, &current, &request.to)?;
            owner_admission(connection, kind, &source.identity, &token, self.origin())?;
            check_fleet_registry_conn(connection, self.origin(), peers, &self.smalltalk.schema_digest())
        };
        let (claim, created) = append_claim_with_commit_context(&self.graph, &input, None, None, None, None,
            ClaimCommitContext { admission: Some(&admission), identity_writer: true, ..Default::default() })?;
        if !created { check_receipt(&self.readers.get(), &claim)?; }
        Ok(receipt(&claim, request, !created))
    }
}

pub fn writer_required() -> St3Error {
    St3Error::new("invalid-rename", "identity naming claims must be written through rename-id")
}

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h40ca3df48b16 owner=fractal_store::store::identities TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS identity_heads (
    identity TEXT PRIMARY KEY, kind TEXT NOT NULL, canonical_id TEXT NOT NULL,
    naming_revision TEXT, unavailable TEXT
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS identity_names (
    name TEXT PRIMARY KEY, identity TEXT NOT NULL, claim_id TEXT
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS identity_names_identity ON identity_names(identity);
CREATE TABLE IF NOT EXISTS identity_dirty (id INTEGER PRIMARY KEY CHECK(id=1));
CREATE TRIGGER IF NOT EXISTS identity_claim_insert AFTER INSERT ON claims
WHEN NEW.kind IN ('agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired','record.repaired')
BEGIN INSERT OR IGNORE INTO identity_dirty VALUES(1); END;
CREATE TRIGGER IF NOT EXISTS identity_claim_delete AFTER DELETE ON claims
WHEN OLD.kind IN ('agent.renamed','mission.renamed','intent.desired','mission.published','agent.retired')
BEGIN INSERT OR IGNORE INTO identity_dirty VALUES(1); END;
CREATE TRIGGER IF NOT EXISTS identity_record_repair AFTER UPDATE OF state ON replica_records
WHEN OLD.state<>NEW.state AND (OLD.state='repaired' OR NEW.state='repaired')
BEGIN INSERT OR IGNORE INTO identity_dirty VALUES(1); END;";

pub fn open(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-hcee25249b3a6 owner=fractal_store::store::identities TODO=trace-statement-and-value-representation */ SCHEMA)?;
    rebuild(transaction)
}

pub fn flush(transaction: &Transaction<'_>) -> Result<()> {
    if transaction.query_row("SELECT EXISTS(SELECT 1 FROM identity_dirty)", [], |row| row.get::<_,bool>(0))? {
        reduce(transaction, false)?;
    }
    Ok(())
}

struct Fact {
    id: String, kind: IdentityKind, identity: String, from: String, to: String,
    previous: Option<String>, declaration: String, origin: String, repaired: bool,
}
struct Head { kind: IdentityKind, canonical: String, revision: Option<String>, blocked: Option<&'static str> }

/// Reduce the naming subgraph, independent of canonical-time or replication arrival order.
/// Only naming/declaration lifecycle changes invoke this fold; unrelated work never does.
pub fn rebuild(transaction: &Transaction<'_>) -> Result<()> {
    reduce(transaction, true)
}

fn reduce(transaction: &Transaction<'_>, clear: bool) -> Result<()> {
    let connection: &Connection = transaction;
    let mut heads = BTreeMap::<String,Head>::new();
    let mut names = BTreeMap::<String,(String,Option<String>)>::new();
    {
        let mut statement = connection.prepare("SELECT subject,kind FROM claims WHERE kind IN ('intent.desired','mission.published') AND subject IN (SELECT subject FROM claims WHERE kind IN ('agent.renamed','mission.renamed') UNION SELECT json_extract(body,'$.fields.to') FROM claims WHERE kind IN ('agent.renamed','mission.renamed') UNION SELECT json_extract(body,'$.fields.from') FROM claims WHERE kind IN ('agent.renamed','mission.renamed')) GROUP BY subject,kind")?;
        for row in statement.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))? {
            let (identity,claim_kind) = row?;
            let kind = if claim_kind == "mission.published" { IdentityKind::Mission } else if identity.starts_with("agent/") { IdentityKind::Agent } else { continue };
            names.insert(identity.clone(),(identity.clone(),None));
            heads.insert(identity.clone(),Head { kind, canonical: identity, revision: None, blocked: None });
        }
    }
    let mut facts = Vec::new();
    {
        let mut statement = connection.prepare("SELECT id,subject,kind,origin,body,EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired') FROM claims WHERE kind IN ('agent.renamed','mission.renamed') ORDER BY id")?;
        for row in statement.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,bool>(5)?)))? {
            let (id,identity,kind,origin,body,repaired) = row?;
            let kind = if kind == AGENT_RENAMED { IdentityKind::Agent } else { IdentityKind::Mission };
            let body: Value = serde_json::from_str(&body)?;
            let fields = &body["fields"];
            heads.entry(identity.clone()).or_insert_with(|| Head { kind, canonical: identity.clone(), revision: None, blocked: Some("pending-replication") });
            names.entry(identity.clone()).or_insert_with(|| (identity.clone(),None));
            let Some((from,to,declaration)) = fields["from"].as_str().zip(fields["to"].as_str()).zip(fields["declaration"].as_str()).map(|((from,to),declaration)| (from,to,declaration)) else {
                heads.get_mut(&identity).expect("inserted head").blocked = Some("conflict"); continue;
            };
            if IdentityKind::of(&identity) != Some(kind) || IdentityKind::of(from) != Some(kind) || IdentityKind::of(to) != Some(kind) || fields["owner_host"].as_str() != Some(origin.as_str()) {
                heads.get_mut(&identity).expect("inserted head").blocked = Some("conflict"); continue;
            }
            facts.push(Fact { id,kind,identity,from:from.into(),to:to.into(),previous:fields["previous_revision"].as_str().map(str::to_owned),declaration:declaration.into(),origin,repaired });
        }
    }
    // Reserve every historical target, even a claim whose predecessor has not arrived. Base
    // storage identities can never be stolen by a rename, irrespective of lexical claim ID.
    let mut winners = BTreeMap::<String,String>::new();
    for fact in &facts {
        if !names.contains_key(&fact.to) { winners.entry(fact.to.clone()).or_insert_with(|| fact.id.clone()); }
    }
    let mut branches = BTreeMap::<(String,Option<String>),Vec<String>>::new();
    for fact in facts.iter().filter(|fact| !fact.repaired) { branches.entry((fact.identity.clone(),fact.previous.clone())).or_default().push(fact.id.clone()); }
    let mut done = BTreeSet::new();
    loop {
        let mut advanced = false;
        for fact in &facts {
            if done.contains(&fact.id) { continue; }
            if fact.repaired { done.insert(fact.id.clone()); continue; }
            let head = heads.get(&fact.identity).expect("fact head");
            if fact.previous.is_some() && fact.previous != head.revision { continue; }
            if fact.previous.is_none() && head.revision.is_some() { continue; }
            let mut reason = None;
            let evidence = claim_by_id_tx(connection, &fact.declaration)?;
            match evidence {
                None => reason = Some("pending-replication"),
                Some(claim) => {
                    if claim.subject != fact.identity || (fact.kind == IdentityKind::Agent && claim.kind != "intent.desired") || (fact.kind == IdentityKind::Mission && claim.kind != "mission.published") || owner_admission(connection,fact.kind,&fact.identity,&fact.declaration,&fact.origin).is_err() {
                        reason = Some("conflict");
                    }
                }
            }
            let declaration_repaired: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=?1 AND state='repaired')", [&fact.declaration], |row|row.get(0))?;
            if declaration_repaired { reason = Some("conflict"); }
            if branches[&(fact.identity.clone(),fact.previous.clone())].len() > 1 { reason = Some("conflict"); }
            if winners.get(&fact.to) != Some(&fact.id) || fact.from != head.canonical { reason = Some("conflict"); }
            let operation_conflict: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM claims c JOIN operations o ON o.id=json_extract(c.body,'$._operation.id') WHERE c.id=?1 AND o.state='conflict')", [&fact.id], |row|row.get(0))?;
            if operation_conflict { reason = Some("conflict"); }
            let head = heads.get_mut(&fact.identity).expect("fact head");
            if let Some(reason) = reason { head.blocked = Some(reason); }
            else if head.blocked != Some("conflict") {
                head.blocked = None; head.canonical.clone_from(&fact.to); head.revision = Some(fact.id.clone());
            }
            done.insert(fact.id.clone()); advanced = true;
        }
        if !advanced { break; }
    }
    let by_id = facts.iter().map(|fact| (fact.id.as_str(),fact)).collect::<BTreeMap<_,_>>();
    for fact in &facts {
        if !done.contains(&fact.id) {
            let head = heads.get_mut(&fact.identity).expect("fact head");
            if head.blocked.is_none() {
                let mut visited = BTreeSet::new();
                let mut previous = fact.previous.as_deref();
                let mut reason = "conflict";
                while let Some(id) = previous {
                    if !visited.insert(id) { break; }
                    let Some(predecessor) = by_id.get(id) else { reason = "pending-replication"; break; };
                    if predecessor.identity != fact.identity || predecessor.repaired { break; }
                    previous = predecessor.previous.as_deref();
                }
                head.blocked = Some(reason);
            }
        }
        if winners.get(&fact.to) == Some(&fact.id) { names.insert(fact.to.clone(),(fact.identity.clone(),Some(fact.id.clone()))); }
    }
    // A received edge can precede its predecessor. Reserve its asserted prior spelling too,
    // so a missing predecessor cannot turn that spelling into an undeclared-recipient bypass.
    // A real target reservation or original storage subject always takes precedence.
    for fact in &facts {
        names.entry(fact.from.clone()).or_insert_with(|| (fact.identity.clone(),Some(fact.id.clone())));
    }
    if clear {
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h4d43fd014a3c owner=fractal_store::store::identities TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM identity_names", [])?;
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h5ef5d37f634d owner=fractal_store::store::identities TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM identity_heads", [])?;
    }
    for (identity,head) in heads {
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h5096ce555101 owner=fractal_store::store::identities TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO identity_heads(identity,kind,canonical_id,naming_revision,unavailable) VALUES (?1,?2,?3,?4,?5)
            ON CONFLICT(identity) DO UPDATE SET kind=excluded.kind,canonical_id=excluded.canonical_id,naming_revision=excluded.naming_revision,unavailable=excluded.unavailable
            WHERE identity_heads.kind IS NOT excluded.kind OR identity_heads.canonical_id IS NOT excluded.canonical_id
                OR identity_heads.naming_revision IS NOT excluded.naming_revision OR identity_heads.unavailable IS NOT excluded.unavailable",
            params![identity,head.kind.label(),head.canonical,head.revision,head.blocked])?;
    }
    for (name,(identity,claim)) in names {
        transaction.execute(/* sqlite-large-value: allow-for-now lv-h6a4d3dc93653 owner=fractal_store::store::identities TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO identity_names(name,identity,claim_id) VALUES (?1,?2,?3)
            ON CONFLICT(name) DO UPDATE SET identity=excluded.identity,claim_id=excluded.claim_id
            WHERE identity_names.identity IS NOT excluded.identity OR identity_names.claim_id IS NOT excluded.claim_id",
            params![name,identity,claim])?;
    }
    transaction.execute(/* sqlite-large-value: allow-for-now lv-h80464fa0fc3c owner=fractal_store::store::identities TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM identity_dirty", [])?;
    Ok(())
}

#[cfg(test)]
#[path = "identities_tests.rs"]
mod tests;
