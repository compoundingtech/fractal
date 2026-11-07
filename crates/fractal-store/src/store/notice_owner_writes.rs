//! Dedicated member-authored writes. An appended row is not proof until sealing succeeds.
use super::*;
use fractal_schema::{notice, notice_owner_ack::{Acknowledgement, Status}, notice_receipt::Receipt};
use super::notice_acknowledgements::{historical_binding, Chain};

fn refused(reason: impl std::fmt::Display) -> FractalError {
    FractalError::new("invalid-notice-owner-proof", format!("notice owner write refused: {reason}"))
}

fn local_owner(store: &Store, binding: &notice_owner::Binding) -> Result<(), FractalError> {
    let key = store.member_key.read().map_err(|error| internal(error.to_string()))?;
    if binding.member != format!("host/{}", store.origin)
        || key.as_ref().is_none_or(|key| key.public() != binding.member_key)
    {
        return Err(refused("configured local member key is not the bound owner"));
    }
    Ok(())
}

impl Store {
    /// Append using the actual owner's member signer, then require durable sealed proof.
    /// A sealing/proof error can leave committed evidence; it never authorizes an unsealed row.
    pub fn acknowledge_notice_owner(&self, acknowledgement: &Acknowledgement) -> Result<ClaimRecord, FractalError> {
        acknowledgement.validate().map_err(refused)?;
        self.seal_local_batches().map_err(internal)?;
        let claim = self.connection.batched(|tx| -> Result<ClaimRecord, FractalError> {
            let chain = Chain::load(tx, None)?;
            if chain.head() != acknowledgement.previous_ack.as_deref() {
                return Err(refused("previous_ack does not match the sealed chain head"));
            }
            let target = historical_binding(tx, &acknowledgement.owner_binding)?;
            local_owner(self, &target)?;
            match acknowledgement.status {
                Status::Active => {
                    if notice_owner::resolve(tx)? != target
                        || Some(notice_authority::binding(tx)?.1.as_str()) != acknowledgement.runtime_binding.as_deref()
                    {
                        return Err(refused("active acknowledgement must bind the selected owner and runtime"));
                    }
                    if chain.active_binding().is_some_and(|prior|
                        prior.incarnation != target.incarnation || prior.member_key != target.member_key)
                    {
                        return Err(refused("former owner must end its active interval before handover"));
                    }
                }
                Status::Ended => {
                    if chain.active_binding() != Some(&target) {
                        return Err(refused("ended acknowledgement must target the prior active binding"));
                    }
                }
            }
            let predecessors = acknowledgement.previous_ack.iter().cloned().collect::<Vec<_>>();
            append_claim_tx(tx, &self.origin, &target.subject, fractal_schema::notice_owner_ack::KIND,
                None, &json!({"fields": acknowledgement}), &predecessors, None).map_err(internal)
        }).map_err(internal)??;
        self.seal_local_batches().map_err(internal)?;
        let connection = self.readers.get();
        let tx = connection.unchecked_transaction().map_err(internal)?;
        // Load the actual durable envelope/signature/position; never synthesize an incoming batch.
        Chain::load(&tx, None)?;
        Ok(claim)
    }

    /// Record immutable delivery evidence as the acknowledged member, not an asserted actor.
    /// Historical receipts do not depend on the current disposable runtime designation.
    pub fn record_notice_receipt(&self, subject: &str, receipt: &Receipt) -> Result<ClaimRecord, FractalError> {
        receipt.validate().map_err(refused)?;
        self.seal_local_batches().map_err(internal)?;
        let claim = self.connection.batched(|tx| -> Result<ClaimRecord, FractalError> {
            let chain = Chain::load(tx, None)?;
            let target = historical_binding(tx, &receipt.owner_binding)?;
            local_owner(self, &target)?;
            let mut fields: BTreeMap<String, Value> = serde_json::from_value(json!(receipt)).map_err(internal)?;
            fields.insert("_registration".into(), json!(notice::HASHES[3]));
            let (hash, manifest, state) = custom::registration(tx, subject).map_err(internal)?
                .ok_or_else(|| refused("receipt registration is unavailable"))?;
            if state != "ready" || hash != notice::HASHES[3]
                || manifest.subject_prefix != notice::RECEIPT_PREFIX {
                return Err(refused("receipt registration is conflicting or mismatched"));
            }
            fractal_schema::registry().validate_subject(subject).map_err(refused)?;
            let spec = manifest.claims.get(fractal_schema::notice_receipt::KIND)
                .ok_or_else(|| refused("receipt kind is not registered"))?;
            fractal_schema::custom::validate_fields(spec, &fields).map_err(refused)?;
            if let Some(reason) = has_unknown_claim_at(tx, subject, None).map_err(internal)? {
                return Err(refused(reason));
            }
            let history = custom::claims(tx, subject).map_err(internal)?;
            match notice_authority::validate(tx, subject, &manifest, fractal_schema::notice_receipt::KIND,
                None, &fields, history.first()) {
                Err(error) if error.downcast_ref::<FractalError>()
                    .is_some_and(|error| error.code == "notice-receipt-authority-unavailable") => {}
                Err(error) => return Err(internal(error)),
                Ok(()) => return Err(refused("receipt validation did not reach the member authority boundary")),
            }
            // Reject competing receipt identities for this source/surface, not just updates
            // at this subject. Read complete canonical histories, including malformed facts.
            let subjects = tx.prepare_cached(
                "SELECT DISTINCT subject FROM claims WHERE subject>=?1 AND subject<?2
                 AND (json_extract(body,'$.fields.source_id')=?3 OR json_extract(body,'$.fields.creation_claim')=?4)
                 AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')"
            ).map_err(internal)?.query_map(params![notice::RECEIPT_PREFIX, "custom/attention/receipt0",
                receipt.source_id, receipt.creation_claim], |row| row.get::<_, String>(0)).map_err(internal)?
                .collect::<rusqlite::Result<Vec<_>>>().map_err(internal)?;
            for candidate in subjects.iter().filter(|candidate| candidate.as_str() != subject) {
                for prior in custom::claims(tx, candidate).map_err(internal)? {
                    let prior_fields = custom::fields(&prior);
                    if prior_fields.get("surface").is_none_or(|surface| Some(surface) == fields.get("surface")) {
                        return Err(refused("conflicting receipt history for this source and surface"));
                    }
                }
            }
            if let [prior] = history.as_slice() {
                if prior.kind == fractal_schema::notice_receipt::KIND && prior.actor.is_none()
                    && custom::fields(prior) == fields {
                    chain.authorize(tx, prior, receipt, None)?;
                    return Ok(prior.clone());
                }
            }
            if !history.is_empty() { return Err(refused("receipt history is immutable or conflicting")); }
            if chain.active_binding() != Some(&target) {
                return Err(refused("receipt owner is not the acknowledged active binding"));
            }
            append_claim_tx(tx, &self.origin, subject, fractal_schema::notice_receipt::KIND,
                None, &json!({"fields": fields}), &[], None).map_err(internal)
        }).map_err(internal)??;
        self.seal_local_batches().map_err(internal)?;
        let connection = self.readers.get();
        let tx = connection.unchecked_transaction().map_err(internal)?;
        Chain::load(&tx, None)?.authorize(&tx, &claim, receipt, None)?;
        Ok(claim)
    }
}

#[cfg(test)]
#[path = "notice_owner_writes_tests.rs"]
mod tests;
