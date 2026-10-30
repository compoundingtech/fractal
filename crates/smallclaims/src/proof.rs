//! Portable, offline rename proofs. Only the caller's fleet anchor is trusted.
//!
//! Envelopes retain their exact sealed CBOR payloads. Writer chains prove a historical
//! cut, not that no later envelope exists; callers pin progress with `after_revision`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::claim::{ClaimRecord, ReplicaEnvelope, ReplicaEnvelopePayload};
use crate::fleet::{FleetClaim, Membership, envelope_signature_message, verify_signature};
use crate::hash::{claim_id_is_content_hash, replica_envelope_hash, verify_replica_batch_header};
use crate::principal::{self, ClaimSignature, Delegation, Facts, Judged, KeyGrant, Verdict};

pub const BUNDLE_SCHEMA: &str = "fractal.rename-proof.v1";
pub const VERDICT_SCHEMA: &str = "fractal.rename-proof-verdict.v1";

/// Redundant descriptive fields are checked against the authenticated claims, never trusted.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenameProofBundle {
    pub schema: String,
    pub fleet_id: String,
    /// Informational only; the verifier compares it to, but never trusts it over, its caller.
    pub fleet_anchor: String,
    pub identity_kind: String,
    pub storage_id: String,
    pub canonical_id: String,
    pub naming_revision: String,
    pub declaration: String,
    pub chain: Vec<String>,
    pub envelopes: Vec<ReplicaEnvelope>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenameProofStep {
    pub revision: String,
    pub from: String,
    pub to: String,
    pub claim_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenameProofVerdict {
    pub schema: String,
    pub ok: bool,
    pub identity_kind: String,
    pub storage_id: String,
    pub canonical_id: String,
    pub naming_revision: String,
    pub declaration: String,
    pub request_actor: String,
    pub kind: String,
    pub storage_identity: String,
    pub from: String,
    pub to: String,
    pub revision: String,
    pub actor: String,
    pub reason: String,
    pub owner_host: String,
    pub signer_member: String,
    pub signer_key: String,
    pub fleet_id: String,
    /// Signed revision tokens and their edges, oldest first.
    pub chain: Vec<RenameProofStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProofError {
    pub code: String,
    pub message: String,
}

impl ProofError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }

    /// Stable process status shared by offline CLI consumers.
    pub fn exit_code(&self) -> i32 {
        match self.code.as_str() {
            "invalid-signature" => 2,
            "broken-chain" => 3,
            "untrusted-membership" => 4,
            "stale-revision" => 5,
            _ => 6,
        }
    }
}

impl fmt::Display for ProofError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ProofError {}

fn malformed(message: impl Into<String>) -> ProofError {
    ProofError::new("malformed-bundle", message)
}
fn chain_error(message: impl Into<String>) -> ProofError {
    ProofError::new("broken-chain", message)
}

struct Record {
    claim: ClaimRecord,
    sequence: u64,
    position: usize,
    signatures: Vec<ClaimSignature>,
}

struct Authentication<'a> {
    records: &'a BTreeMap<String, Record>,
    membership: &'a Membership,
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
        let fields = &claim.body["fields"];
        let judged = Judged {
            id: &claim.id, subject: &claim.subject, kind: &claim.kind,
            actor: claim.actor.as_deref(),
            content: principal::content_digest(&claim.subject, &claim.kind, claim.actor.as_deref(), &claim.body),
            fields,
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

    fn require(&self, id: &str) -> Result<(), ProofError> {
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

fn order(record: &Record) -> (u128, &str, u64, &str, usize, &str) {
    let claim = &record.claim;
    (claim.accepted_at_unix_ms, &claim.origin, record.sequence, &claim.batch_id, record.position, &claim.id)
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
        subject.strip_prefix("host/").is_some_and(|writer| {
            self.authentication.membership.incarnations().any(|incarnation| {
                incarnation.name == writer && incarnation.member_key == key
            })
        })
    }

    fn revoked_before(&self, principal: &str, key: &str) -> bool {
        self.authentication.records.values().any(|record| {
            record.claim.kind == principal::KEY_REVOKED
                && record.claim.subject == principal
                && record.claim.body["fields"]["key"].as_str() == Some(key)
                && order(record) < order(self.record)
                && self.authentication.verdict(&record.claim.id, self.depth + 1) == Verdict::Verified
                && record.signatures.iter().any(|signature| {
                    signature.signer == principal || principal::Family::of(&signature.signer) == Some(principal::Family::Node)
                })
        })
    }

    fn nonce_used_before(&self, key: &str, nonce: &str) -> bool {
        self.authentication.records.values().any(|record| {
            order(record) < order(self.record)
                && record.signatures.iter().any(|signature| signature.key == key && signature.nonce == nonce)
        })
    }
}

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
    let mut records = BTreeMap::<String, Record>::new();
    let mut fleet = Vec::new();
    let mut writers = BTreeMap::<&str, BTreeMap<u64, &ReplicaEnvelope>>::new();
    let mut batch_hashes = BTreeMap::new();
    for envelope in &bundle.envelopes {
        if envelope.sequence == 0 || writers.entry(&envelope.writer).or_default().insert(envelope.sequence, envelope).is_some() {
            return Err(chain_error("duplicate writer sequence or zero sequence"));
        }
        let bytes = envelope.payload.bytes().map_err(|error| malformed(error.to_string()))?;
        if replica_envelope_hash(&envelope.writer, envelope.sequence, envelope.previous_hash.as_deref(), envelope.accepted_at_unix_ms, bytes) != envelope.hash {
            return Err(ProofError::new("invalid-signature", "envelope content hash mismatch"));
        }
        let (Some(key), Some(signature)) = (&envelope.member_key, &envelope.signature) else {
            return Err(ProofError::new("invalid-signature", "envelope is not sealed by a member key"));
        };
        if !verify_signature(key, &envelope_signature_message(&bundle.fleet_id, &envelope.writer, envelope.sequence, &envelope.hash), signature) {
            return Err(ProofError::new("invalid-signature", "invalid envelope signature"));
        }
        let mut cursor = std::io::Cursor::new(bytes);
        let payload: ReplicaEnvelopePayload = ciborium::from_reader(&mut cursor).map_err(|error| malformed(error.to_string()))?;
        if cursor.position() != bytes.len() as u64 { return Err(malformed("trailing envelope payload bytes")); }
        verify_replica_batch_header(&payload.batch).map_err(|error| malformed(error.to_string()))?;
        batch_hashes.insert((envelope.writer.as_str(), envelope.sequence), payload.batch.hash.clone());
        let batch = &payload.batch;
        if batch.origin != envelope.writer || batch.replica_sequence != envelope.sequence || batch.previous_hash != envelope.previous_hash || batch.accepted_at_unix_ms != envelope.accepted_at_unix_ms {
            return Err(malformed("envelope and batch header disagree"));
        }
        for (hash, blob) in &payload.blobs {
            if hex::encode(Sha256::digest(blob)) != *hash {
                return Err(ProofError::new("invalid-signature", "blob content hash mismatch"));
            }
        }
        for (position, claim) in batch.claims.iter().enumerate() {
            if claim.batch_id != batch.id || claim.origin != batch.origin || claim.accepted_at_unix_ms != batch.accepted_at_unix_ms || !claim_id_is_content_hash(claim).map_err(|error| malformed(error.to_string()))? {
                return Err(ProofError::new("invalid-signature", "claim identity or batch metadata mismatch"));
            }
            validate_rename_schema(claim)?;
            if claim.kind.starts_with("fleet.") {
                fleet.push(FleetClaim {
                    id: claim.id.clone(), kind: claim.kind.clone(), subject: claim.subject.clone(),
                    fields: serde_json::from_value(claim.body["fields"].clone()).map_err(|error| malformed(error.to_string()))?,
                    writer: envelope.writer.clone(), sequence: envelope.sequence,
                    signers: BTreeSet::from([key.clone()]),
                });
            }
            let signatures = payload.claim_signatures.get(&claim.id).cloned().into_iter().collect();
            if records.insert(claim.id.clone(), Record { claim: claim.clone(), sequence: batch.replica_sequence, position, signatures }).is_some() {
                return Err(malformed("duplicate claim ID"));
            }
        }
        if payload.claim_signatures.keys().any(|id| !batch.claims.iter().any(|claim| &claim.id == id)) {
            return Err(malformed("signature does not name a claim in its envelope"));
        }
    }
    for envelopes in writers.values() {
        let mut previous = None;
        for (index, envelope) in envelopes.values().enumerate() {
            if envelope.sequence != index as u64 + 1 || envelope.previous_hash.as_deref() != previous {
                return Err(chain_error("writer envelope chain is incomplete or disconnected"));
            }
            previous = batch_hashes.get(&(envelope.writer.as_str(), envelope.sequence)).map(String::as_str);
        }
    }
    let membership = Membership::fold(Some(fleet_anchor), &fleet);
    for envelope in &bundle.envelopes {
        let key = envelope.member_key.as_deref().expect("checked above");
        // The anchor's admission bootstraps its own writer window.
        let bootstrap = key == fleet_anchor && fleet.iter().any(|claim| {
            claim.writer == envelope.writer && claim.sequence == envelope.sequence
                && claim.kind == "fleet.member-admitted" && membership.counts(&claim.id)
                && claim.fields.get("member_key").and_then(Value::as_str) == Some(fleet_anchor)
        });
        if !bootstrap && !membership.keys_for(&envelope.writer, envelope.sequence).contains(key) {
            return Err(ProofError::new("untrusted-membership", format!("writer {} sequence {} is outside its member window", envelope.writer, envelope.sequence)));
        }
    }
    let authentication = Authentication { records: &records, membership: &membership };
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

/// Resolve an agent's historical spelling only through an authenticated, complete naming
/// chain on the same storage subject. A matching string on an unrelated subject is not evidence.
fn authenticated_actor_alias(
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
        let mut current = candidate;
        let mut expected = requested;
        let mut visited = BTreeSet::new();
        loop {
            let claim = &current.claim;
            if !visited.insert(&claim.id) { return Err(chain_error("actor alias cycle")); }
            if claim.kind != "agent.renamed" || claim.subject != storage {
                return Err(chain_error("actor alias crosses storage identity"));
            }
            authentication.require(&claim.id)?;
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
fn validate_rename_schema(claim: &ClaimRecord) -> Result<(), ProofError> {
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
        if let Some(value) = request.get(name).filter(|value| !value.is_null()) {
            if !value.is_string() || value != &fields[field] {
                return Err(chain_error(format!("request {name} disagrees with effect")));
            }
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
#[cfg(test)]
mod tests {
    use super::*;

    fn empty_bundle() -> RenameProofBundle {
        RenameProofBundle {
            schema: BUNDLE_SCHEMA.into(), fleet_id: "example-fleet".into(),
            fleet_anchor: "caller-anchor".into(),
            identity_kind: "mission".into(), storage_id: "mission/example".into(),
            canonical_id: "mission/renamed".into(), naming_revision: "claim/head".into(),
            declaration: "claim/declaration".into(), chain: vec!["claim/head".into()],
            envelopes: Vec::new(),
        }
    }

    #[test]
    fn failure_exit_codes_are_stable() {
        for (code, expected) in [
            ("invalid-signature", 2), ("broken-chain", 3), ("untrusted-membership", 4),
            ("stale-revision", 5), ("malformed-bundle", 6),
        ] {
            let error = ProofError::new(code, "example");
            assert_eq!(error.exit_code(), expected);
            let encoded = serde_json::to_value(&error).expect("error encodes");
            assert_eq!(encoded["code"], code);
        }
    }

    #[test]
    fn metadata_without_authenticated_material_is_not_a_proof() {
        let error = verify_rename_proof(&empty_bundle(), "caller-anchor", None)
            .expect_err("metadata is not evidence");
        assert_eq!(error.code, "broken-chain");
    }

    #[test]
    fn unknown_schema_and_empty_anchor_fail_closed() {
        let mut bundle = empty_bundle();
        assert_eq!(verify_rename_proof(&bundle, "", None).unwrap_err().code, "malformed-bundle");
        bundle.schema = "fractal.rename-proof.v999".into();
        assert_eq!(verify_rename_proof(&bundle, "caller-anchor", None).unwrap_err().code, "malformed-bundle");
    }

    #[test]
    fn embedded_anchor_cannot_override_caller_anchor() {
        let mut bundle = empty_bundle();
        bundle.fleet_anchor = "attacker-anchor".into();
        assert_eq!(verify_rename_proof(&bundle, "caller-anchor", None).unwrap_err().code, "untrusted-membership");
    }

    #[test]
    fn naming_subjects_use_the_runtime_subject_grammar() {
        assert!(valid_subject("agent/example/worker", "agent"));
        assert!(valid_subject("mission/example", "mission"));
        for invalid in ["agent/", "agent//worker", "agent/example/", "agent/white space", "mission/example"] {
            assert!(!valid_subject(invalid, "agent"), "{invalid}");
        }
        assert!(!valid_subject(&format!("agent/{}", "x".repeat(512)), "agent"));
    }
}
