//! Authenticated per-owner acknowledgement chain; no fleet-wide timestamp order is inferred.
use super::*;
use fractal_schema::notice_owner_ack::{Acknowledgement, Status};
use super::notice_owner_proof::{self, Position};

fn unavailable(reason: impl std::fmt::Display) -> FractalError {
    FractalError::new("notice-owner-proof-unavailable", format!("notice owner acknowledgement unavailable: {reason}"))
}
fn invalid(reason: impl std::fmt::Display) -> FractalError {
    FractalError::new("invalid-notice-owner-proof", format!("invalid notice owner acknowledgement: {reason}"))
}

pub(super) fn historical_binding(connection: &Connection, id: &str) -> Result<notice_owner::Binding, FractalError> {
    let claim = claim_by_id_tx(connection, id).map_err(internal)?
        .ok_or_else(|| unavailable("owner declaration is not admitted"))?;
    if claim.kind != "intent.desired" || !claim.subject.starts_with("notice-owner/")
        || claim.body["kind"] != "notice-owner" || !claim.body["member"].is_null()
        || ["owner_run", "owner_generation", "owner_step"].iter().any(|key| !claim.body[key].is_null()) {
        return Err(invalid("owner binding is not a standalone declaration"));
    }
    if !custom::claims(connection, &claim.subject).map_err(internal)?.iter().any(|row| row.id == id) {
        return Err(invalid("owner declaration was repaired away"));
    }
    let declaration = fractal_schema::notice_owner::Declaration::from_desired(&claim.body["desired"])
        .map_err(invalid)?;
    let membership = smallclaims::store::fleet_membership_tx(connection).map_err(internal)?;
    let mut members = membership.incarnations().filter(|member| member.admitted_claim == declaration.incarnation
        && format!("host/{}", member.name) == declaration.member);
    let member = members.next().ok_or_else(|| unavailable("owner admission is missing"))?;
    if members.next().is_some() { return Err(invalid("ambiguous owner admission")); }
    Ok(notice_owner::Binding { declaration_claim: claim.id, subject: claim.subject,
        member: declaration.member, incarnation: declaration.incarnation, member_key: member.member_key.clone() })
}

struct Node {
    claim: ClaimRecord,
    acknowledgement: Acknowledgement,
    binding: notice_owner::Binding,
    position: Position,
}

pub(super) struct Chain { nodes: Vec<Node> }

impl Chain {
    pub(super) fn load(connection: &Connection, incoming: Option<(&ReplicaBatch, &ClaimRecord)>) -> Result<Self, FractalError> {
        let blocked: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records WHERE state IN ('unknown','invalid')
             AND (kind_hint='notice.owner-acknowledged' OR subject_hint GLOB 'notice-owner/*')
             AND (?1 IS NULL OR claim_id IS NOT ?1)
             AND (state='invalid' OR ?1 IS NULL OR kind_hint IS NOT 'notice.owner-acknowledged'))",
            [incoming.map(|(_, claim)| claim.id.as_str())], |row| row.get(0),
        ).map_err(internal)?;
        if blocked { return Err(unavailable("owner prerequisites are quarantined")); }
        let mut claims = connection.prepare(/* sqlite-large-value: allow-for-now lv-h04e9d719f7b7 owner=fractal_store::store::notice_acknowledgements TODO=trace-statement-and-value-representation */ &format!(
            "SELECT {CLAIM_COLUMNS} FROM claims WHERE kind='notice.owner-acknowledged'
             AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')"
        )).map_err(internal)?.query_map([], claim_from_row).map_err(internal)?
            .collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
        if let Some((_, claim)) = incoming {
            claims.retain(|stored| stored.id != claim.id);
            claims.push(claim.clone());
        }
        let mut nodes = BTreeMap::new();
        let mut successors = BTreeMap::<Option<String>, Vec<String>>::new();
        for claim in claims {
            if claim.actor.is_some() { return Err(invalid("acknowledgement must be member-authored, not actor-asserted")); }
            let acknowledgement = Acknowledgement::from_fields(&custom::fields(&claim)).map_err(invalid)?;
            let binding = historical_binding(connection, &acknowledgement.owner_binding)?;
            if claim.subject != binding.subject { return Err(invalid("owner acknowledgement subject mismatch")); }
            let batch = incoming.filter(|(_, candidate)| candidate.id == claim.id).map(|(batch, _)| batch);
            let position = notice_owner_proof::authenticate(connection, &claim, batch, &binding.member_key)?;
            if position.writer != binding.member.trim_start_matches("host/") {
                return Err(invalid("owner writer does not match bound incarnation"));
            }
            if let Some(runtime) = acknowledgement.runtime_binding.as_deref() {
                let runtime = claim_by_id_tx(connection, runtime).map_err(internal)?
                    .ok_or_else(|| unavailable("runtime declaration is not admitted"))?;
                if runtime.kind != "intent.desired" || runtime.body["kind"] != "agent"
                    || !runtime.body["desired"]["children"].as_array().is_some_and(|children|
                        children.iter().any(|child| child["name"] == "notice-runtime")) {
                    return Err(invalid("acknowledged runtime is not a designated agent declaration"));
                }
                if !custom::claims(connection, &runtime.subject).map_err(internal)?.iter().any(|row| row.id == runtime.id) {
                    return Err(invalid("acknowledged runtime declaration was repaired away"));
                }
            }
            successors.entry(acknowledgement.previous_ack.clone()).or_default().push(claim.id.clone());
            nodes.insert(claim.id.clone(), Node { claim, acknowledgement, binding, position });
        }
        if nodes.is_empty() { return Ok(Self { nodes: Vec::new() }); }
        if successors.values().any(|children| children.len() != 1) { return Err(invalid("forked acknowledgement chain")); }
        let mut previous = None;
        let mut ordered: Vec<Node> = Vec::with_capacity(nodes.len());
        while let Some(next) = successors.remove(&previous).and_then(|mut ids| ids.pop()) {
            let node = nodes.remove(&next).ok_or_else(|| invalid("cyclic acknowledgement chain"))?;
            if let Some(prior) = ordered.last() {
                match (&prior.acknowledgement.status, &node.acknowledgement.status) {
                    (Status::Active, Status::Ended) if node.binding.declaration_claim == prior.binding.declaration_claim => {}
                    (Status::Active, Status::Active) if node.binding.incarnation == prior.binding.incarnation
                        && node.binding.member_key == prior.binding.member_key => {}
                    (Status::Ended, Status::Active) => {}
                    _ => return Err(invalid("owner handover requires the former owner's ended acknowledgement")),
                }
            } else if node.acknowledgement.status != Status::Active {
                return Err(invalid("initial owner acknowledgement must be active"));
            }
            if ordered.iter().rev().find(|prior| prior.binding.incarnation == node.binding.incarnation)
                .is_some_and(|prior| (node.position.sequence, node.position.position)
                    <= (prior.position.sequence, prior.position.position)) {
                return Err(invalid("acknowledgements do not advance the owner's causal position"));
            }
            previous = Some(next);
            ordered.push(node);
        }
        if !nodes.is_empty() { return Err(unavailable("acknowledgement predecessor is missing or cyclic")); }
        Ok(Self { nodes: ordered })
    }

    pub(super) fn head(&self) -> Option<&str> { self.nodes.last().map(|node| node.claim.id.as_str()) }

    pub(super) fn authorize(&self, connection: &Connection, claim: &ClaimRecord,
        receipt: &fractal_schema::notice_receipt::Receipt, batch: Option<&ReplicaBatch>) -> Result<(), FractalError> {
        if claim.actor.is_some() { return Err(invalid("receipt must be member-authored, not actor-asserted")); }
        if !self.nodes.iter().any(|node| node.acknowledgement.status == Status::Active
            && node.acknowledgement.owner_binding == receipt.owner_binding) {
            return Err(unavailable("the receipt owner's active acknowledgement is not admitted"));
        }
        let binding = historical_binding(connection, &receipt.owner_binding)?;
        let position = notice_owner_proof::authenticate(connection, claim, batch, &binding.member_key)?;
        if position.writer != binding.member.trim_start_matches("host/") { return Err(invalid("wrong receipt owner writer")); }
        let mut active = None;
        for node in &self.nodes {
            if node.binding.incarnation == binding.incarnation
                && (node.position.sequence, node.position.position) < (position.sequence, position.position) {
                active = if node.acknowledgement.status == Status::Active {
                    Some(node.binding.declaration_claim.as_str())
                } else { None };
            }
        }
        if active != Some(receipt.owner_binding.as_str()) {
            return Err(invalid("owner binding was not active at the receipt's causal position"));
        }
        Ok(())
    }

    pub(super) fn active_binding(&self) -> Option<&notice_owner::Binding> {
        self.nodes.last().filter(|node| node.acknowledgement.status == Status::Active).map(|node| &node.binding)
    }

    pub(super) fn fence(&self, connection: &Connection) -> Result<Fence, FractalError> {
        let owner = notice_owner::resolve(connection)?;
        let (_, runtime) = notice_authority::binding(connection)?;
        let latest = self.nodes.last().filter(|node| node.acknowledgement.status == Status::Active
            && node.binding == owner && node.acknowledgement.runtime_binding.as_deref() == Some(runtime.as_str()))
            .ok_or_else(|| unavailable("selected owner/runtime change has not been acknowledged"))?;
        Ok(Fence { owner_binding: owner.declaration_claim, acknowledgement: latest.claim.id.clone(), runtime_binding: runtime })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Fence { pub owner_binding: String, pub acknowledgement: String, pub runtime_binding: String }

impl Store {
    pub fn notice_owner_fence(&self) -> Result<Fence, FractalError> {
        let connection = self.readers.get();
        let tx = connection.unchecked_transaction().map_err(internal)?;
        Chain::load(&tx, None)?.fence(&tx)
    }
}
