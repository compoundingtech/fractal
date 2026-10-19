//! The part of `store::custom` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use serde::{Deserialize, Serialize};
use st3_schema::custom::{self as schema, Authority, Basis, Expr, Manifest, Predicate, Select};
pub use st3_store::store::custom::*;
pub use st3_store::store::custom::{RegistrationRequest, ReplyRequest, attention_item, create_schema, flush, open, prepare, rebuild, source};
#[cfg(test)]
mod tests;

pub trait CustomStore {
    fn custom_attention_items(
            &self,
            person: Option<&str>,
        ) -> Result<Vec<AttentionItemView>>;
}

impl CustomStore for Store {

    fn custom_attention_items(
            &self,
            person: Option<&str>,
        ) -> Result<Vec<AttentionItemView>> {
        let connection = self.readers.get();
        let rows = if let Some(person) = person {
            connection.prepare("SELECT body,requested_at FROM custom_sources WHERE active=1 AND person=?1 ORDER BY subject")?
                .query_map([person], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            connection
                .prepare(
                    "SELECT body,requested_at FROM custom_sources WHERE active=1 ORDER BY subject",
                )?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        rows.into_iter()
            .map(|(text, at)| {
                attention_item(&serde_json::from_str(&text)?, at.parse().unwrap_or(0))
            })
            .collect()
    }
}

