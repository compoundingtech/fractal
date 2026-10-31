//! Full authenticated naming-chain reduction and caller-pinned revision progress.
use std::collections::{BTreeMap, BTreeSet};
use serde_json::Value;
use super::{BUNDLE_SCHEMA, VERDICT_SCHEMA, RenameProofBundle, RenameProofStep, RenameProofVerdict, ProofError, chain_error, malformed, verify_sealed_history};
use super::authentication::{Authentication, authenticated_actor_alias};

/// Authenticate a historical rename cut without opening a store, network, or key directory.
/// `after_revision`, when supplied, must be a token strictly before the proven head.
pub fn verify_rename_proof(
    bundle: &RenameProofBundle,
    fleet_anchor: &str,
    after_revision: Option<&str>,
) -> Result<RenameProofVerdict, ProofError> {
    if bundle.disclosure != "fleet-internal" {
        return Err(malformed("rename evidence requires fleet-internal disclosure classification"));
    }
    if bundle.schema != BUNDLE_SCHEMA || bundle.fleet_id.is_empty() || fleet_anchor.is_empty() {
        return Err(malformed("unsupported schema or empty fleet identity/anchor"));
    }
    if bundle.fleet_anchor != fleet_anchor {
        return Err(ProofError::new("untrusted-membership", "informational anchor disagrees with caller anchor"));
    }
    let (rename_kind, declaration_kind, prefix) = match bundle.identity_kind.as_str() {
        "agent" => ("agent.renamed", "intent.desired", "agent/"),
        "mission" => ("mission.renamed", "mission.published", "mission/"),
        _ => return Err(malformed("unknown identity kind")),
    };
    let history = verify_sealed_history(&bundle.envelopes, &bundle.fleet_id, fleet_anchor)?;
    let records = &history.records;
    let membership = &history.membership;
    let authentication = Authentication { records, membership };
    let head = records.get(&bundle.naming_revision).ok_or_else(|| chain_error("missing naming head"))?;
    let storage_id = &head.claim.subject;
    if !storage_id.starts_with(prefix) { return Err(chain_error("head has the wrong identity kind")); }
    let mut reversed = Vec::new();
    let mut visited = BTreeSet::new();
    let mut current = Some(bundle.naming_revision.as_str());
    while let Some(id) = current {
        if !visited.insert(id) { return Err(chain_error("naming revision cycle")); }
        let record = records.get(id).ok_or_else(|| chain_error("missing previous revision"))?;
        if record.claim.kind != rename_kind || &record.claim.subject != storage_id { return Err(chain_error("cross-identity previous revision")); }
        authentication.require(id)?;
        super::authentication::validate_rename_schema(&record.claim)?;
        if !crate::naming::admits_replicated(&record.claim).map_err(|error| malformed(error.to_string()))? {
            return Err(chain_error("naming claim disagrees with immutable runtime admission"));
        }
        let fields = &record.claim.body["fields"];
        let request_actor = fields["request"]["actor"].as_str()
            .ok_or_else(|| chain_error("rename request has no actor"))?;
        let actor = record.claim.actor.as_deref()
            .ok_or_else(|| chain_error("rename has no actor"))?;
        if fields["owner_host"].as_str() != Some(record.claim.origin.as_str())
            || !authenticated_actor_alias(&authentication, request_actor, actor)?
            || fields["request"]["to"] != fields["to"]
            || fields["request"]["reason"] != fields["reason"]
            || fields["reason"].as_str().is_none_or(|reason| reason.trim().is_empty())
        {
            return Err(chain_error("rename request, owner or actor is inconsistent"));
        }
        current = match fields.get("previous_revision") {
            None | Some(Value::Null) => None,
            Some(Value::String(previous)) => Some(previous),
            _ => return Err(chain_error("invalid previous revision token")),
        };
        reversed.push(id.to_owned());
    }
    reversed.reverse();
    let mut canonical = storage_id.as_str();
    let mut names = BTreeSet::from([canonical]);
    let mut final_declaration = None;
    for id in &reversed {
        let claim = &records[id].claim;
        let fields = &claim.body["fields"];
        let to = fields["to"].as_str().ok_or_else(|| chain_error("missing rename target"))?;
        let requested_from = fields["request"]["from"].as_str()
            .ok_or_else(|| malformed("missing request source"))?;
        if !names.contains(requested_from) {
            return Err(chain_error("request source is not an authenticated spelling of this storage identity"));
        }
        if fields["from"].as_str() != Some(canonical) || !to.starts_with(prefix) || !names.insert(to) { return Err(chain_error("rename spellings do not form a non-cyclic chain")); }
        let declaration = fields["declaration"].as_str().ok_or_else(|| chain_error("missing declaration token"))?;
        require_declaration(&authentication, declaration, storage_id, declaration_kind, &claim.origin)?;
        canonical = to;
        final_declaration = Some(declaration);
    }
    let projection = naming_projection(&authentication, bundle);
    if !projection.accepted
        || projection.heads.get(storage_id).is_none_or(|head| head.blocked.is_some())
    {
        return Err(chain_error("authenticated naming facts fence this identity or revision"));
    }
    if bundle.storage_id != *storage_id || bundle.canonical_id != canonical || final_declaration != Some(bundle.declaration.as_str()) || bundle.chain != reversed {
        return Err(malformed("bundle metadata disagrees with authenticated naming chain"));
    }
    if let Some(after) = after_revision
        && !reversed.iter().take(reversed.len().saturating_sub(1)).any(|id| id == after) {
        return Err(ProofError::new("stale-revision", "required revision is absent or is not strictly before the head"));
    }
    let fields = &head.claim.body["fields"];
    let actor = fields["request"]["actor"].as_str()
        .ok_or_else(|| chain_error("head request has no actor"))?;
    let envelope = bundle.envelopes.iter().find(|envelope| {
        envelope.writer == head.claim.origin && envelope.sequence == head.sequence
    }).ok_or_else(|| chain_error("head has no envelope"))?;
    let chain = reversed.iter().map(|id| {
        let fields = &records[id].claim.body["fields"];
        RenameProofStep {
            revision: id.clone(), claim_id: id.clone(),
            from: fields["from"].as_str().expect("validated chain").into(),
            to: fields["to"].as_str().expect("validated chain").into(),
        }
    }).collect();
    Ok(RenameProofVerdict {
        schema: VERDICT_SCHEMA.into(), ok: true, identity_kind: bundle.identity_kind.clone(),
        disclosure: bundle.disclosure.clone(),
        storage_id: storage_id.clone(), canonical_id: canonical.into(), naming_revision: head.claim.id.clone(),
        declaration: bundle.declaration.clone(), chain, request_actor: actor.into(),
        kind: bundle.identity_kind.clone(), storage_identity: storage_id.clone(),
        from: fields["from"].as_str().expect("validated source").into(), to: canonical.into(),
        revision: head.claim.id.clone(), actor: actor.into(),
        reason: fields["reason"].as_str().expect("validated reason").into(),
        owner_host: head.claim.origin.clone(), signer_member: envelope.writer.clone(),
        signer_key: envelope.member_key.clone().expect("validated envelope signer"),
        fleet_id: bundle.fleet_id.clone(),
    })
}

fn require_declaration(
    authentication: &Authentication<'_>, declaration: &str, identity: &str, kind: &str, origin: &str,
) -> Result<(), ProofError> {
    let mut evidence = authentication.records.get(declaration).ok_or_else(|| chain_error("missing declaration"))?;
    let mut visited = BTreeSet::new();
    loop {
        let claim = &evidence.claim;
        if !visited.insert(&claim.id) || claim.subject != identity || claim.kind != kind {
            return Err(chain_error("invalid ownership declaration"));
        }
        authentication.require(&claim.id)?;
        if kind == "mission.published" {
            return if claim.origin == origin { Ok(()) } else { Err(chain_error("rename writer does not own mission declaration")) };
        }
        match claim.body["kind"].as_str() {
            Some("agent") if claim.body["member"]["host"].as_str() == Some(origin) => return Ok(()),
            Some("stop") if claim.predecessors.len() == 1 => {
                evidence = authentication.records.get(&claim.predecessors[0])
                    .ok_or_else(|| chain_error("missing agent ownership predecessor"))?;
            }
            _ => return Err(chain_error("agent declaration has no ownership evidence")),
        }
    }
}

fn admitted_naming(claim: &crate::claim::ClaimRecord) -> bool {
    crate::naming::admits_replicated(claim).unwrap_or(false)
        && super::authentication::validate_rename_schema(claim).is_ok()
}

/// Only principal-authenticated facts can reserve names or fence another identity.
fn naming_projection(authentication: &Authentication<'_>, bundle: &RenameProofBundle) -> crate::naming::Projection<()> {
    use crate::naming::{Fact, Head};
    let mut heads = BTreeMap::new();
    let mut names = BTreeMap::new();
    let repaired_records = authentication.records.values().filter_map(|record| {
        let claim = &record.claim;
        (claim.kind == "record.repaired" && authentication.require(&claim.id).is_ok()
            && claim.body["fields"]["replacement"].as_str().is_some_and(|id| authentication.records.contains_key(id)))
            .then(|| claim.body["fields"]["record"].as_str()).flatten()
    }).collect::<BTreeSet<_>>();
    let repaired = authentication.records.values().filter_map(|record| {
        let envelope = bundle.envelopes.iter().find(|envelope| envelope.writer == record.claim.origin && envelope.sequence == record.sequence)?;
        let reference = crate::hash::replica_record_ref(&envelope.writer, envelope.sequence, &envelope.hash, record.position as u64);
        repaired_records.contains(reference.as_str()).then_some(record.claim.id.as_str())
    }).collect::<BTreeSet<_>>();
    let mut operations = BTreeMap::<String, crate::operation_metadata::OperationMetadata>::new();
    for record in authentication.records.values() {
        let claim = &record.claim;
        if !repaired.contains(claim.id.as_str()) && authentication.require(&claim.id).is_ok()
            && admitted_naming(claim)
            && let Some((operation, digest)) = crate::store::operation_parts(&claim.body) {
            operations.entry(operation.into()).or_default().include(digest.into(), claim.id.clone(), true);
        }
    }
    let conflicts = operations.into_iter().filter_map(|(id, metadata)| {
        metadata.row().filter(|(_, _, state)| state == "conflict").map(|_| id)
    }).collect::<BTreeSet<_>>();
    for record in authentication.records.values() {
        let claim = &record.claim;
        if (claim.kind == "mission.published" || (claim.kind == "intent.desired" && claim.subject.starts_with("agent/")))
            && authentication.require(&claim.id).is_ok()
        {
            names.insert(claim.subject.clone(), (claim.subject.clone(), None));
            heads.insert(claim.subject.clone(), Head { kind: (), canonical: claim.subject.clone(), revision: None, blocked: None });
        }
    }
    let mut facts = Vec::new();
    for record in authentication.records.values() {
        let claim = &record.claim;
        let declaration_kind = match claim.kind.as_str() {
            "agent.renamed" => "intent.desired", "mission.renamed" => "mission.published", _ => continue,
        };
        if authentication.require(&claim.id).is_err() || !admitted_naming(claim) { continue; }
        let fields = &claim.body["fields"];
        let identity = &claim.subject;
        heads.entry(identity.clone()).or_insert_with(|| Head { kind: (), canonical: identity.clone(), revision: None, blocked: Some("pending-replication") });
        names.entry(identity.clone()).or_insert_with(|| (identity.clone(), None));
        let declaration = fields["declaration"].as_str().expect("validated naming schema");
        let unavailable = if fields["owner_host"].as_str() != Some(claim.origin.as_str())
            || repaired.contains(declaration)
            || crate::store::operation_parts(&claim.body).is_some_and(|(id, _)| conflicts.contains(id))
            || require_declaration(authentication, declaration, identity, declaration_kind, &claim.origin).is_err()
        { Some("conflict") } else { None };
        facts.push(Fact {
            id: claim.id.clone(), identity: identity.clone(),
            from: fields["from"].as_str().expect("validated naming schema").into(),
            to: fields["to"].as_str().expect("validated naming schema").into(),
            previous: fields["previous_revision"].as_str().map(str::to_owned), repaired: repaired.contains(claim.id.as_str()), unavailable,
        });
    }
    crate::naming::reduce(heads, names, facts, Some(&bundle.naming_revision))
}

#[cfg(test)]
#[path = "rename_tests.rs"]
mod tests;

