//! The part of `store::declarations` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use serde::Deserialize;
pub use crate::model::DeclarationDiff;
pub use fractal_delivery::store::declarations::*;
pub use fractal_delivery::store::declarations::{PublicationDefinition, diffs};

pub trait DeclarationsStore {
    fn publication_definition(
            &self,
            subject: &str,
            at: u64,
        ) -> Result<Option<PublicationDefinition>, St3Error>;
}

impl DeclarationsStore for Store {

    fn publication_definition(
            &self,
            subject: &str,
            at: u64,
        ) -> Result<Option<PublicationDefinition>, St3Error> {
        selected(&self.readers.get(), subject, Some(at))
    }
}

