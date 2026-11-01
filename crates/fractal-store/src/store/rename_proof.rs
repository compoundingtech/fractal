//! Portable rename evidence is built from the real signed replication history.
use super::*;
use super::identities::IdentityKind;
use smallclaims::proof::RenameProofBundle;

impl Store {
    /// Export the selected naming revision, or an exact historical claim-ID revision.
    /// Names (including historical aliases) resolve to the immutable storage subject.
    pub fn export_rename_proof(
        &self,
        kind: IdentityKind,
        id: &str,
        revision: Option<&str>,
    ) -> Result<RenameProofBundle, St3Error> {
        let connection = self.readers.get();
        let resolved = identities::resolve_identity_conn(&connection, kind, id)?;
        let head = resolved.naming_revision.as_deref().ok_or_else(||
            St3Error::new("broken-chain", "identity has no rename revision to prove"))?;
        let mut chain = Vec::new();
        let mut declarations = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut cursor = Some(head.to_owned());
        let mut canonical_id = None;
        let mut declaration = None;
        let mut selected = revision.is_none();
        while let Some(token) = cursor {
            if !seen.insert(token.clone()) {
                return Err(St3Error::new("broken-chain", "rename revision chain contains a cycle"));
            }
            let claim = claim_by_id_tx(&connection, &token).map_err(internal)?
                .ok_or_else(|| St3Error::new("broken-chain", "rename revision is missing"))?;
            if claim.kind != kind.claim_kind() || claim.subject != resolved.identity {
                return Err(St3Error::new("broken-chain", "rename revision does not belong to the identity"));
            }
            let fields = &claim.body["fields"];
            if revision == Some(token.as_str()) { selected = true; }
            if selected {
                let claim_declaration = fields["declaration"].as_str()
                    .ok_or_else(|| St3Error::new("malformed-bundle", "rename declaration is missing"))?;
                declarations.insert(claim_declaration.to_owned());
                if canonical_id.is_none() {
                    canonical_id = Some(fields["to"].as_str()
                        .ok_or_else(|| St3Error::new("malformed-bundle", "rename target is missing"))?.to_owned());
                    declaration = Some(claim_declaration.to_owned());
                }
                chain.push(token);
            }
            cursor = fields["previous_revision"].as_str().map(str::to_owned);
        }
        if !selected {
            return Err(St3Error::new("stale-revision", "requested revision is absent from the selected naming chain"));
        }
        for token in &declarations {
            if claim_by_id_tx(&connection, token).map_err(internal)?.is_none() {
                return Err(St3Error::new("broken-chain", "rename declaration evidence is missing"));
            }
        }
        chain.reverse();
        let naming_revision = chain.last().cloned()
            .ok_or_else(|| St3Error::new("broken-chain", "rename chain is empty"))?;
        drop(connection);
        let fleet_id = self.graph.bound_fleet().map_err(internal)?
            .ok_or_else(|| St3Error::new("untrusted-membership", "rename proofs require a bound fleet"))?;
        let anchor = self.graph.fleet_anchor().map_err(internal)?
            .ok_or_else(|| St3Error::new("untrusted-membership", "rename proofs require a pinned fleet anchor"))?;
        let bundle = RenameProofBundle {
            schema: "fractal.rename-proof.v1".into(),
            disclosure: "fleet-internal".into(),
            fleet_id,
            fleet_anchor: anchor.clone(),
            identity_kind: match kind { IdentityKind::Agent => "agent", IdentityKind::Mission => "mission" }.into(),
            storage_id: resolved.identity,
            canonical_id: canonical_id.ok_or_else(|| St3Error::new("broken-chain", "rename target is missing"))?,
            naming_revision,
            declaration: declaration.ok_or_else(|| St3Error::new("broken-chain", "rename declaration is missing"))?,
            chain,
            envelopes: self.graph.rename_proof_envelopes()
                .map_err(|error| St3Error::new("untrusted-membership", error.to_string()))?,
        };
        smallclaims::proof::verify_rename_proof(&bundle, &anchor, None)
            .map_err(|error| {
                let code = match error.code.as_str() {
                    "invalid-signature" => "invalid-signature",
                    "broken-chain" => "broken-chain",
                    "untrusted-membership" => "untrusted-membership",
                    "stale-revision" => "stale-revision",
                    _ => "malformed-bundle",
                };
                St3Error::new(code, error.message)
            })?;
        Ok(bundle)
    }
}
