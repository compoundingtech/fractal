//! The part of `store::arrangements` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use fractal_schema::arrangements::{self as schema, Operation};
pub use fractal_delivery::store::arrangements::*;
pub use fractal_delivery::store::arrangements::{Head, SCHEMA, arrangement_at, arrangements_at, bump_inventory_revisions, flush, open, prepare, project, project_replicated, rebuild, register};

pub trait ArrangementsStore {
    fn arrangements(&self, person: &str, through: u64) -> Result<Vec<Value>>;
    /// Owner-local projection counter, including retired and not-yet-created heads.
    /// It commits with the heads and is independent of claim admission indices.
    fn arrangement_inventory_revision(&self, person: &str) -> Result<u64>;
    fn arrangement(&self, subject: &str, through: u64) -> Result<Option<Value>>;
    fn arrangements_changed(&self, after: u64, through: u64) -> Result<bool>;
    fn edit_arrangement(&self, input: &ClaimInput, expected_subjects: &BTreeMap<String,String>) -> Result<ClaimRecord,St3Error>;
}

impl ArrangementsStore for Store {

    fn arrangements(&self, person: &str, through: u64) -> Result<Vec<Value>> {
        self.read_snapshot(|_| arrangements_at(&self.readers.get(),person,through))
    }

    fn arrangement_inventory_revision(&self, person: &str) -> Result<u64> {
        self.read_snapshot(|_| Ok(self.readers.get().query_row(
            "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM meta
             WHERE key='arrangement_inventory_revision_v1/'||?1),0)",
            [person], |row| row.get(0))?))
    }

    fn arrangement(&self, subject: &str, through: u64) -> Result<Option<Value>> {
        self.read_snapshot(|_| arrangement_at(&self.readers.get(),subject,through))
    }

    fn arrangements_changed(&self, after: u64, through: u64) -> Result<bool> {
        Ok(self.readers.get().query_row("SELECT EXISTS(SELECT 1 FROM arrangements WHERE changed_index>?1 AND changed_index<=?2)",params![after,through.min(i64::MAX as u64)],|row| row.get(0))?)
    }

    fn edit_arrangement(&self, input: &ClaimInput, expected_subjects: &BTreeMap<String,String>) -> Result<ClaimRecord,St3Error> {
        if input.kind != "arrangement.edited" { return Err(St3Error::new("invalid-arrangement-operations","edit_arrangement requires arrangement.edited")); }
        let expected_subjects = {
            let connection = self.readers.get();
            let mut stable = BTreeMap::new();
            for (subject, revision) in expected_subjects {
                let identity = fractal_store::store::identities::resolve_reference_conn(&connection, subject)?;
                if stable.insert(identity, revision.clone()).is_some_and(|previous| previous != *revision) {
                    return Err(St3Error::new("stale-fence", "alias references have conflicting declaration fences"));
                }
            }
            stable
        };
        let mut stable_input;
        let actor = input.actor.as_deref().map(|actor|
            fractal_store::store::identities::resolve_reference_conn(&self.readers.get(), actor)
        ).transpose()?;
        let input = if actor == input.actor { input } else {
            stable_input = input.clone();
            stable_input.actor = actor;
            &stable_input
        };
        if let Some(actor) = &input.actor { self.graph.ensure_principal_key(actor)?; }
        if input.expected_subject.is_some() {
            let mut merging = input.clone();
            merging.expected_subject = None;
            return append_claim_with_subject_fences(&self.graph,&merging,None,None,Some(&expected_subjects),None).map(|(claim,_)| claim);
        }
        append_claim_with_subject_fences(&self.graph,input,None,None,Some(&expected_subjects),None).map(|(claim,_)| claim)
    }
}

