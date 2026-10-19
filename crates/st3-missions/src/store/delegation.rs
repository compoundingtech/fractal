//! The part of `store::delegation` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::model::{DelegationPolicyRequest, DelegationProof};
pub use st3_delivery::store::delegation::*;
pub use st3_delivery::store::delegation::{ACTIONS, message_sent, policy_actions, validate_write};

pub fn add_fields(fields: &mut BTreeMap<String, Value>, proof: &DelegationProof) {
    fields.insert("acted_for".into(), json!(proof.person));
    fields.insert("delegation".into(), json!(proof));
}

pub fn add_evidence(
    connection: &Connection,
    evidence: &mut Vec<String>,
    proof: &DelegationProof,
) -> Result<(), St3Error> {
    let instruction = message_sent(connection, &proof.message)?;
    for reference in [&instruction.id, &proof.policy] {
        if !evidence.contains(reference) {
            evidence.push(reference.clone());
        }
    }
    Ok(())
}

pub trait DelegationStore {
    fn delegated_update_request(
            &self,
            proof: &DelegationProof,
        ) -> Result<ClaimRecord, St3Error>;
    fn delegation_evidence(
            &self,
            evidence: &mut Vec<String>,
            proof: &DelegationProof,
        ) -> Result<(), St3Error>;
    fn set_delegation_policy(
            &self,
            input: &DelegationPolicyRequest,
        ) -> Result<ClaimRecord, St3Error>;
}

impl DelegationStore for Store {

    fn delegated_update_request(
            &self,
            proof: &DelegationProof,
        ) -> Result<ClaimRecord, St3Error> {
        let request = claim_by_id_tx(&self.readers.get(), &proof.episode)
            .map_err(internal)?
            .filter(|claim| {
                claim.kind == "work.person-asked"
                    && claim.body["fields"]["person"] == proof.person
                    && person_work::is_update(claim)
            })
            .ok_or_else(|| {
                denied("only an informational update may be closed through attention")
            })?;
        Ok(request)
    }

    fn delegation_evidence(
            &self,
            evidence: &mut Vec<String>,
            proof: &DelegationProof,
        ) -> Result<(), St3Error> {
        add_evidence(&self.readers.get(), evidence, proof)
    }

    fn set_delegation_policy(
            &self,
            input: &DelegationPolicyRequest,
        ) -> Result<ClaimRecord, St3Error> {
        self.append_claim(&ClaimInput {
            subject: input.person.clone(),
            kind: "person.delegation-set".into(),
            actor: Some(input.actor.clone()),
            fields: BTreeMap::from([("actions".into(), json!(input.actions))]),
            evidence: input.evidence.clone(),
            expected_subject: None,
            idempotency_key: Some(input.idempotency_key.clone()),
        })
    }
}

