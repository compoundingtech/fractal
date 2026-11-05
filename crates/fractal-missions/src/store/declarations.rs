//! The part of `store::declarations` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use serde::Deserialize;
pub use crate::model::DeclarationDiff;
pub use fractal_delivery::store::declarations::*;
pub use fractal_delivery::store::declarations::{PublicationDefinition, diffs};

pub trait DeclarationsStore {
    fn mission_declaration(
        &self, mission: &str, revision: Option<&str>, at: u64,
    ) -> Result<Option<(MissionSpec, u64)>, St3Error>;
    fn publication_definition(
            &self,
            subject: &str,
            at: u64,
        ) -> Result<Option<PublicationDefinition>, St3Error>;
}

impl DeclarationsStore for Store {
    fn mission_declaration(
        &self, mission: &str, revision: Option<&str>, at: u64,
    ) -> Result<Option<(MissionSpec, u64)>, St3Error> {
        let connection = self.readers.get();
        let mission = crate::store::missions::mission_storage_id(&connection, mission)?;
        let row: Option<(String, String)> = connection.query_row(
            "SELECT r.body, c.accepted_at_unix_ms FROM mission_revisions r
             JOIN claims c ON c.id=r.claim_id
             LEFT JOIN mission_definitions d ON d.mission_id=r.mission_id
             WHERE r.mission_id=?1 AND r.revision=COALESCE(?2,d.revision)
             AND r.created_index<=?3",
            params![mission, revision, at], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(internal)?;
        row.map(|(body, published)| Ok((serde_json::from_str(&body).map_err(internal)?,
            published.parse().map_err(internal)?))).transpose()
    }


    fn publication_definition(
            &self,
            subject: &str,
            at: u64,
        ) -> Result<Option<PublicationDefinition>, St3Error> {
        selected(&self.readers.get(), subject, Some(at))
    }
}

