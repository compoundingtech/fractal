//! Principal authorization and portable naming-schema policy over authenticated history.
use std::collections::{BTreeMap, BTreeSet};
use serde_json::Value;
use crate::claim::ClaimRecord;
use crate::fleet::Membership;
use crate::principal::{self, Delegation, Facts, Judged, KeyGrant, Verdict};
use super::history::{Record, VerifiedHistory};
use super::{ProofError, chain_error, malformed};

pub(super) struct Authentication<'a> {
    pub(super) records: &'a BTreeMap<String, Record>,
    pub(super) membership: &'a Membership,
}

impl Authentication<'_> {
    fn verdict(&self, id: &str, depth: usize) -> Verdict {
        if depth > 64 {
            return Verdict::Invalid("delegation or revocation cycle".into());
        }
        let Some(record) = self.records.get(id) else {
            return Verdict::Held(format!("missing claim {id}"));
        };
        let claim = &record.claim;
        let fields = principal::facts::signature_fields(&claim.kind, &claim.body);
        let judged = Judged {
            id: &claim.id, subject: &claim.subject, kind: &claim.kind,
            actor: claim.actor.as_deref(),
            content: principal::content_digest(&claim.subject, &claim.kind, claim.actor.as_deref(), &claim.body),
            fields: &fields,
        };
        let facts = ProofFacts { authentication: self, record, depth };
        let mut result = Verdict::Unsigned;
        for signature in &record.signatures {
            if matches!(claim.kind.as_str(), "agent.renamed" | "mission.renamed")
                && signature.format.is_some()
            {
                continue;
            }
            let verdict = principal::judge(&judged, Some(signature), &facts);
            if verdict == Verdict::Verified {
                return verdict;
            }
            result = verdict;
        }
        result
    }

    pub(super) fn require(&self, id: &str) -> Result<(), ProofError> {
        match self.verdict(id, 0) {
            Verdict::Verified => Ok(()),
            Verdict::Held(reason) => Err(ProofError::new("untrusted-membership", reason)),
            verdict => Err(ProofError::new("invalid-signature", format!("claim {id}: {verdict:?}"))),
        }
    }
}

struct ProofFacts<'a, 'b> {
    authentication: &'a Authentication<'b>,
    record: &'a Record,
    depth: usize,
}

fn before(left: &Record, right: &Record) -> bool {
    use principal::facts::{OrderComponent, compare_order};
    fn components(record: &Record) -> [OrderComponent<'_>; 6] {
        let claim = &record.claim;
        [
            OrderComponent::Unsigned(claim.accepted_at_unix_ms),
            OrderComponent::Text(&claim.origin),
            OrderComponent::Integer(i128::from(record.sequence)),
            OrderComponent::Text(&claim.batch_id),
            OrderComponent::Integer(record.position as i128),
            OrderComponent::Text(&claim.id),
        ]
    }
    compare_order(components(left), components(right)).is_lt()
}

impl Facts for ProofFacts<'_, '_> {
    fn delegation(&self, claim_id: &str) -> Option<Delegation> {
        let record = self.authentication.records.get(claim_id)?;
        if record.claim.kind != principal::KEY_GRANTED { return None; }
        Some(Delegation {
            subject: record.claim.subject.clone(),
            grant: KeyGrant::from_fields(&record.claim.body["fields"] )?,
            verdict: self.authentication.verdict(claim_id, self.depth + 1),
        })
    }

    fn is_root(&self, subject: &str, key: &str) -> bool {
        principal::facts::root_eligible(subject, key, principal::facts::RootContext::Record {
            membership: self.authentication.membership,
            writer: &self.record.claim.origin,
            sequence: self.record.sequence,
            claim_id: &self.record.claim.id,
        })
    }

    fn revoked_before(&self, principal: &str, key: &str) -> bool {
        self.authentication.records.values().any(|record| {
            record.claim.kind == principal::KEY_REVOKED
                && record.claim.subject == principal
                && record.claim.body["fields"]["key"].as_str() == Some(key)
                && before(record, self.record)
                && self.authentication.verdict(&record.claim.id, self.depth + 1) == Verdict::Verified
                && record.signatures.iter().any(|signature| {
                    signature.signer == principal || principal::Family::of(&signature.signer) == Some(principal::Family::Node)
                })
        })
    }

    fn nonce_used_before(&self, key: &str, nonce: &str) -> bool {
        self.authentication.records.values().any(|record| {
            before(record, self.record)
                && record.signatures.iter().any(|signature| {
                    let claim = &record.claim;
                    signature.key == key && signature.nonce == nonce
                        && principal::facts::nonce_eligible(
                            &claim.id, &claim.subject, &claim.kind, claim.actor.as_deref(),
                            &claim.body, signature,
                        )
                })
        })
    }
}

/// Resolve an agent's historical spelling only through an authenticated, complete naming
/// chain on the same storage subject. A matching string on an unrelated subject is not evidence.
pub(super) fn authenticated_actor_alias(
    authentication: &Authentication<'_>,
    requested: &str,
    storage: &str,
) -> Result<bool, ProofError> {
    if requested == storage { return Ok(true); }
    if !requested.starts_with("agent/") || !storage.starts_with("agent/") { return Ok(false); }
    for candidate in authentication.records.values().filter(|record| {
        record.claim.kind == "agent.renamed" && record.claim.subject == storage
            && record.claim.body["fields"]["to"].as_str() == Some(requested)
    }) {
        if authentication.require(&candidate.claim.id).is_err() { continue; }
        let mut current = candidate;
        let mut expected = requested;
        let mut visited = BTreeSet::new();
        loop {
            let claim = &current.claim;
            if !visited.insert(&claim.id) { return Err(chain_error("actor alias cycle")); }
            if claim.kind != "agent.renamed" || claim.subject != storage {
                return Err(chain_error("actor alias crosses storage identity"));
            }
            let fields = &claim.body["fields"];
            if fields["to"].as_str() != Some(expected)
                || fields["owner_host"].as_str() != Some(claim.origin.as_str()) {
                return Err(chain_error("actor alias spelling or owner mismatch"));
            }
            let declaration = fields["declaration"].as_str()
                .and_then(|id| authentication.records.get(id))
                .ok_or_else(|| chain_error("missing actor alias declaration"))?;
            let mut evidence = declaration;
            let mut predecessors = BTreeSet::new();
            loop {
                if !predecessors.insert(&evidence.claim.id)
                    || evidence.claim.subject != storage || evidence.claim.kind != "intent.desired" {
                    return Err(chain_error("invalid actor alias ownership chain"));
                }
                authentication.require(&evidence.claim.id)?;
                match evidence.claim.body["kind"].as_str() {
                    Some("agent") => {
                        if evidence.claim.body["member"]["host"].as_str() != Some(claim.origin.as_str()) {
                            return Err(chain_error("actor alias writer does not own declaration"));
                        }
                        break;
                    }
                    Some("stop") if evidence.claim.predecessors.len() == 1 => {
                        evidence = authentication.records.get(&evidence.claim.predecessors[0])
                            .ok_or_else(|| chain_error("missing actor alias ownership predecessor"))?;
                    }
                    _ => return Err(chain_error("actor alias declaration has no owner")),
                }
            }
            expected = fields["from"].as_str().ok_or_else(|| chain_error("actor alias has no source"))?;
            match fields.get("previous_revision") {
                None | Some(Value::Null) => return Ok(expected == storage),
                Some(Value::String(previous)) => {
                    current = authentication.records.get(previous)
                        .ok_or_else(|| chain_error("missing actor alias previous revision"))?;
                    authentication.require(previous)?;
                }
                _ => return Err(chain_error("malformed actor alias previous revision")),
            }
        }
    }
    Ok(false)
}


fn valid_subject(subject: &str, family: &str) -> bool {
    subject.len() <= 512
        && !subject.chars().any(|character| character.is_whitespace() || character.is_control())
        && subject.strip_prefix(family).and_then(|suffix| suffix.strip_prefix('/'))
            .is_some_and(|suffix| !suffix.is_empty() && !suffix.split('/').any(str::is_empty))
}

/// The portable subset of the runtime's rename schema, without a dependency from
/// generic smallclaims back into the Fractal runtime/schema crates.
pub(super) fn validate_rename_schema(claim: &ClaimRecord) -> Result<(), ProofError> {
    let family = match claim.kind.as_str() {
        "agent.renamed" => "agent",
        "mission.renamed" => "mission",
        _ => return Ok(()),
    };
    let fields = &claim.body["fields"];
    if !valid_subject(&claim.subject, family) { return Err(malformed("invalid naming subject")); }
    for name in ["from", "to"] {
        if !fields[name].as_str().is_some_and(|subject| valid_subject(subject, family)) {
            return Err(malformed(format!("invalid naming {name}")));
        }
    }
    for name in ["reason", "owner_host", "declaration"] {
        if fields[name].as_str().is_none() { return Err(malformed(format!("missing naming {name}"))); }
    }
    for name in ["previous_revision", "preserved_incarnation"] {
        if fields.get(name).is_some_and(|value| !value.is_string()) {
            return Err(malformed(format!("invalid naming {name}")));
        }
    }
    let request = &fields["request"];
    if !request.is_object() { return Err(malformed("rename request must be an object")); }
    for name in ["from", "to", "actor", "reason", "idempotency_key"] {
        if request[name].as_str().is_none() { return Err(malformed(format!("missing request {name}"))); }
    }
    for name in ["from", "to"] {
        if !valid_subject(request[name].as_str().expect("checked string"), family) {
            return Err(malformed(format!("invalid request {name}")));
        }
    }
    for (name, field) in [("expected_naming_revision", "previous_revision"), ("expected_declaration", "declaration")] {
        if let Some(value) = request.get(name).filter(|value| !value.is_null())
            && (!value.is_string() || value != &fields[field]) {
            return Err(chain_error(format!("request {name} disagrees with effect")));
        }
    }
    let key = request["idempotency_key"].as_str().expect("checked string");
    if key.is_empty() || key.len() > 512 || key.chars().any(char::is_control) {
        return Err(malformed("invalid rename idempotency key"));
    }
    let operation = crate::store::operation_parts(&claim.body)
        .ok_or_else(|| malformed("missing rename operation binding"))?;
    let digest = crate::hash::canonical_hash(&(
        "st3.identity-rename-request.v1", claim.kind.as_str(), crate::hash::canonical_json_value(request),
    )).map_err(|error| malformed(error.to_string()))?;
    if operation.0 != crate::store::operation_id_for_key(key) || operation.1 != digest {
        return Err(chain_error("rename request operation binding mismatch"));
    }
    Ok(())
}

impl VerifiedHistory {
    /// Verify a claim's signature and principal delegation using only this anchored history.
    /// Node signatures require the signer's own authenticated writer window; signatures merely
    /// carried by another writer cannot establish that window. Nonces count only when signed
    /// genuinely for their claim.
    pub fn verify_claim(&self, id: &str) -> Result<(), ProofError> {
        Authentication { records: &self.records, membership: &self.membership }.require(id)
    }

    /// Resolve a historical agent spelling through authenticated same-storage naming edges.
    pub fn verify_actor_alias(&self, requested: &str, storage: &str) -> Result<bool, ProofError> {
        authenticated_actor_alias(&Authentication { records: &self.records, membership: &self.membership }, requested, storage)
    }

    /// Validate the portable runtime schema and request/effect binding of all naming claims.
    pub fn validate_rename_claims(&self) -> Result<(), ProofError> {
        for record in self.records.values() { validate_rename_schema(&record.claim)?; }
        Ok(())
    }
}

#[cfg(test)]
#[path = "authentication_tests.rs"]
mod tests;
