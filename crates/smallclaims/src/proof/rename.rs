//! Full authenticated naming-chain reduction and caller-pinned revision progress.
use std::collections::BTreeSet;
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
    history.validate_rename_claims()?;
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
        let evidence = records.get(declaration).ok_or_else(|| chain_error("missing declaration"))?;
        if evidence.claim.subject != *storage_id || evidence.claim.kind != declaration_kind {
            return Err(chain_error("declaration does not belong to storage identity"));
        }
        authentication.require(declaration)?;
        if bundle.identity_kind == "mission" && evidence.claim.origin != claim.origin { return Err(chain_error("rename writer does not own mission declaration")); }
        if bundle.identity_kind == "agent" {
            let mut owner_evidence = evidence;
            let mut owner_visited = BTreeSet::new();
            loop {
                if !owner_visited.insert(&owner_evidence.claim.id) {
                    return Err(chain_error("declaration predecessor cycle"));
                }
                if owner_evidence.claim.subject != *storage_id || owner_evidence.claim.kind != declaration_kind {
                    return Err(chain_error("invalid agent ownership declaration"));
                }
                authentication.require(&owner_evidence.claim.id)?;
                match owner_evidence.claim.body["kind"].as_str() {
                    Some("agent") => {
                        if owner_evidence.claim.body["member"]["host"].as_str() != Some(claim.origin.as_str()) {
                            return Err(chain_error("rename writer does not own agent declaration"));
                        }
                        break;
                    }
                    Some("stop") if owner_evidence.claim.predecessors.len() == 1 => {
                        owner_evidence = records.get(&owner_evidence.claim.predecessors[0])
                            .ok_or_else(|| chain_error("missing agent ownership predecessor"))?;
                    }
                    _ => return Err(chain_error("agent declaration has no ownership evidence")),
                }
            }
        }
        canonical = to;
        final_declaration = Some(declaration);
    }
    if bundle.storage_id != *storage_id || bundle.canonical_id != canonical || final_declaration != Some(bundle.declaration.as_str()) || bundle.chain != reversed {
        return Err(malformed("bundle metadata disagrees with authenticated naming chain"));
    }
    if let Some(after) = after_revision {
        if !reversed.iter().take(reversed.len().saturating_sub(1)).any(|id| id == after) {
            return Err(ProofError::new("stale-revision", "required revision is absent or is not strictly before the head"));
        }
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

#[cfg(test)]
#[path = "rename_tests.rs"]
mod tests;
