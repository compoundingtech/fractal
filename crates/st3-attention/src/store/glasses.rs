//! The part of `store::glasses` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use st3_schema::glasses::MAX_GLASSES;
pub use st3_store::store::glasses::*;
use st3_store::store::validate_replicated_claim;
pub use st3_store::store::glasses::{glasses_at, prepare};

#[cfg(test)]
mod tests {
    use super::super::tests::{exchange_from, exchange_of, receive_and_project};
    use super::*;
    use crate::model::ReplicationInventory;
    use st3_store::store::glasses_tests_shared_attention_tests::{input};

    #[test]
    fn glass_owner_admission_and_generic_projections_are_private() {
        let store = Store::open_memory("alder").unwrap();
        for actor in [None, Some("agent/worker"), Some("person/alex")] {
            let mut claim = input(1, "Private");
            claim.actor = actor.map(str::to_owned);
            assert_eq!(
                store.append_claim(&claim).unwrap_err().code,
                "glass-owner-forbidden"
            );
        }
        assert_eq!(
            store
                .append_client_claim(&input(1, "Bypass"))
                .unwrap_err()
                .code,
            "claim-write-forbidden"
        );
        let first = store.append_claim(&input(1, "Private")).unwrap();
        assert!(
            store
                .claims_page(None, None, 0, None, false, 100)
                .unwrap()
                .claims
                .is_empty()
        );
        assert!(
            store
                .status_history(Some(&first.subject), None, None)
                .unwrap()
                .subjects
                .is_empty()
        );
        assert!(
            store
                .glasses("person/alex", i64::MAX as u64)
                .unwrap()
                .is_empty()
        );
        let connection = store.readers.get();
        let batch=connection.query_row("SELECT id, origin, replica_sequence, previous_hash, hash, accepted_at_unix_ms FROM batches WHERE id=?1",[&first.batch_id],|r| Ok(crate::model::ReplicaBatch { id:r.get(0)?,origin:r.get(1)?,replica_sequence:r.get(2)?,previous_hash:r.get(3)?,hash:r.get(4)?,accepted_at_unix_ms:r.get::<_,String>(5)?.parse().unwrap(), claims:vec![] })).unwrap();
        let mut forged = first.clone();
        forged.actor = Some("agent/worker".into());
        forged.id = claim_hash(
            &forged.batch_id,
            &forged.subject,
            &forged.kind,
            &forged.origin,
            forged.actor.as_deref(),
            &forged.body,
            &forged.predecessors,
        )
        .unwrap();
        assert_eq!(
            validate_replicated_claim(&connection, &batch, &forged)
                .err()
                .unwrap()
                .code,
            "glass-owner-forbidden"
        );
    }
}
