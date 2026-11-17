//! Nonmutating admission and edge evidence over one SQLite reader snapshot.
use super::*;

pub(super) fn plan(store: &Store, kind: IdentityKind, request: &RenameRequest, peers: &[String]) -> Result<RenamePlan, FractalError> {
    let request_value = serde_json::to_value(request).map_err(internal)?;
    let mut plan = RenamePlan {
        kind, identity: None, from: request.from.clone(), to: request.to.clone(),
        current_canonical_id: None, naming_revision: None, declaration: None,
        request: request.clone(), request_digest: request_digest(kind.claim_kind(), &request_value)?,
        admissible: false, refusal: None,
    };
    let connection = store.readers.get();
    let snapshot = connection.unchecked_transaction().map_err(internal)?;
    let admission = (|| {
        validate_request(kind, request)?;
        if let Some((_, claim_id, state)) = operation_tx(&snapshot, &operation_id_for_key(&request.idempotency_key)).map_err(internal)? {
            if state == "conflict" { return Err(FractalError::new("idempotency-conflict", "rename operation has conflicting requests")); }
            let claim = claim_by_id_tx(&snapshot, &claim_id).map_err(internal)?.ok_or_else(|| unavailable(&request.from, "missing"))?;
            if claim.kind != kind.claim_kind() || claim.body["fields"]["request"] != request_value {
                return Err(FractalError::new("idempotency-mismatch", "idempotency key identifies a different request"));
            }
            check_receipt(&snapshot, &claim)?;
            let fields = &claim.body["fields"];
            plan.identity = Some(claim.subject.clone());
            plan.current_canonical_id = fields["from"].as_str().map(str::to_owned);
            plan.naming_revision = fields["previous_revision"].as_str().map(str::to_owned);
            plan.declaration = fields["declaration"].as_str().map(str::to_owned);
            return Ok(());
        }
        let source = resolve_identity_conn(&snapshot, kind, &request.from)?;
        plan.identity = Some(source.identity.clone());
        plan.current_canonical_id = Some(source.canonical_id.clone());
        plan.naming_revision = source.naming_revision.clone();
        plan.declaration = declaration_token(&snapshot, kind, &source.identity)?;
        let declaration = plan.declaration.as_deref().ok_or_else(|| unavailable(&source.identity, "missing"))?;
        resolve_reference_conn(&snapshot, &request.actor)?;
        rename_admission(&snapshot, store, kind, request, &source, declaration, peers)
    })();
    match admission {
        Ok(()) => plan.admissible = true,
        Err(error) if matches!(error.code, "internal" | "read-deadline") || error.is_sqlite_contention() => return Err(error),
        Err(error) => plan.refusal = Some(RenameRefusal { code: error.code.into(), detail: error.message }),
    }
    Ok(plan)
}

fn availability(error: &FractalError) -> RenameProofAvailability {
    match error.details.get("reason").and_then(Value::as_str) {
        Some("conflict" | "operation-conflict-naming-refold-limit") => RenameProofAvailability::Conflicted,
        Some("pending-replication" | "missing") => RenameProofAvailability::PendingReplication,
        _ if error.code == "identity-unknown" => RenameProofAvailability::NotFound,
        _ => RenameProofAvailability::Unavailable,
    }
}

pub(super) fn proof(store: &Store, kind: IdentityKind, from: &str, to: &str, digest: Option<&str>) -> Result<RenameProof, FractalError> {
    validate_name(kind, from)?;
    validate_name(kind, to)?;
    let connection = store.readers.get();
    let snapshot = connection.unchecked_transaction().map_err(internal)?;
    let snapshot_index = snapshot.query_row("SELECT COALESCE(MAX(store_index),0) FROM claims", [], |row| row.get(0)).map_err(internal)?;
    let mut proof = RenameProof {
        kind, identity: None, from: from.into(), to: to.into(), claim_id: None,
        naming_revision: None, current_canonical_id: None, declaration: None,
        previous_revision: None, request_digest: None, operation_id: None, actor: None,
        owner_host: None, accepted_at_unix_ms: None, availability: RenameProofAvailability::NotFound,
        snapshot_index, claim: None,
    };
    // Always consult both native resolvers. A matching raw claim can never override a fence.
    let source = resolve_identity_conn(&snapshot, kind, from);
    let target = resolve_identity_conn(&snapshot, kind, to);
    let mut failure = None;
    for result in [&source, &target] {
        if let Err(error) = result {
            if matches!(error.code, "internal" | "read-deadline") || error.is_sqlite_contention() {
                return Err(FractalError { code: error.code, message: error.message.clone(), details: error.details.clone() });
            }
            let state = availability(error);
            // Conflicts dominate pending evidence, which dominates missing names.
            let rank = |state| match state {
                RenameProofAvailability::Conflicted => 3,
                RenameProofAvailability::PendingReplication => 2,
                RenameProofAvailability::Unavailable => 1,
                _ => 0,
            };
            if failure.is_none_or(|previous| rank(state) > rank(previous)) { failure = Some(state); }
        }
    }
    if let Some(state) = failure { proof.availability = state; return Ok(proof); }
    let source = source?;
    let target = target?;
    if source.identity != target.identity { return Ok(proof); }
    proof.identity = Some(source.identity.clone());
    proof.naming_revision = source.naming_revision.clone();
    proof.current_canonical_id = Some(source.canonical_id.clone());
    // Walk the selected chain, rather than trusting a raw from/to claim or a reserved alias.
    let mut revision = source.naming_revision.clone();
    let mut visited = BTreeSet::new();
    while let Some(id) = revision {
        if !visited.insert(id.clone()) { proof.availability = RenameProofAvailability::Conflicted; return Ok(proof); }
        let Some(claim) = claim_by_id_tx(&snapshot, &id).map_err(internal)? else {
            proof.availability = RenameProofAvailability::PendingReplication;
            return Ok(proof);
        };
        if claim.subject != source.identity || claim.kind != kind.claim_kind() {
            proof.availability = RenameProofAvailability::Conflicted;
            return Ok(proof);
        }
        let fields = &claim.body["fields"];
        revision = fields["previous_revision"].as_str().map(str::to_owned);
        if fields["from"].as_str() != Some(from) || fields["to"].as_str() != Some(to) { continue; }
        if digest.is_some_and(|expected| claim.request_digest.as_deref() != Some(expected)) { return Ok(proof); }
        proof.claim_id = Some(claim.id.clone());
        proof.declaration = fields["declaration"].as_str().map(str::to_owned);
        proof.previous_revision = revision;
        proof.request_digest = claim.request_digest.clone();
        proof.operation_id = claim.operation_id.clone();
        proof.actor = claim.actor.clone();
        proof.owner_host = fields["owner_host"].as_str().map(str::to_owned);
        proof.accepted_at_unix_ms = Some(claim.accepted_at_unix_ms);
        proof.availability = if source.naming_revision.as_deref() == Some(claim.id.as_str()) {
            RenameProofAvailability::Selected
        } else { RenameProofAvailability::Superseded };
        proof.claim = Some(claim);
        return Ok(proof);
    }
    Ok(proof)
}
