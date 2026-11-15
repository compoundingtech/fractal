//! Header-only seeks over accepted person sends. The claims log owns the bodies;
//! this projection adds no prompt journal and cannot query across agents.
use super::*;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS input_history_author_agent ON claims(
         actor,json_extract(body,'$.fields.to'),store_index DESC)
         WHERE kind='message.sent' AND json_extract(body,'$.fields.from')=actor;
         CREATE INDEX IF NOT EXISTS input_history_author_conversation ON claims(
         actor,json_extract(body,'$.fields.to'),json_extract(body,'$.fields.session_id'),store_index DESC)
         WHERE kind='message.sent' AND json_extract(body,'$.fields.from')=actor;",
    )?;
    Ok(())
}
static SQL: std::sync::LazyLock<[String; 2]> = std::sync::LazyLock::new(|| {
    [("input_history_author_agent", "AND ?5 IS NULL"),
     ("input_history_author_conversation", "AND json_extract(claims.body,'$.fields.session_id')=?5")]
        .map(|(index, filter)| format!(
            "SELECT claims.subject,claims.id,claims.actor,
             json_extract(claims.body,'$.fields.to'),
             json_extract(claims.body,'$.fields.session_id'),claims.store_index,
             ({UNREPAIRED_CLAIM})
             FROM claims INDEXED BY {index}
             WHERE claims.kind='message.sent' AND claims.actor=?1
             AND json_extract(claims.body,'$.fields.to')=?2
             AND json_extract(claims.body,'$.fields.from')=claims.actor
             AND claims.store_index<=?3 AND claims.store_index<?4
             {filter} ORDER BY claims.store_index DESC LIMIT ?6"))
});


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub message_id: String,
    pub claim_id: String,
    pub author: String,
    pub agent: String,
    pub conversation: Option<String>,
    pub accepted_order: u64,
}

/// `through` freezes insert membership; current repairs/deletions still win.
pub struct Query<'a> {
    pub person: &'a str,
    pub agent: &'a str,
    pub conversation: Option<&'a str>,
    pub through: u64,
    pub before: Option<u64>,
    pub limit: usize,
}

/// A bounded candidate window, not a count of visible history or a text search.
/// Continue after `examined_through`, even if authorization removed every row.
#[derive(Debug)]
pub struct Page {
    pub entries: Vec<Candidate>,
    pub examined_through: Option<u64>,
    pub has_more_candidates: bool,
}

impl Store {
    /// The caller checks current source authorization/eligibility before any header
    /// leaves this read. Errors abort the whole page; false excludes that source.
    /// Body resolution, search and signed API cursors belong to the client read API.
    pub fn input_history_candidates(
        &self,
        query: &Query<'_>,
        mut authorized: impl FnMut(&Candidate) -> Result<bool>,
    ) -> Result<Page> {
        anyhow::ensure!(query.person.starts_with("person/") && query.person.len() > 7,
            "input history requires a person");
        anyhow::ensure!(query.agent.starts_with("agent/") && query.agent.len() > 6,
            "input history requires one agent");
        anyhow::ensure!((1..=200).contains(&query.limit), "history window must be 1 through 200");
        anyhow::ensure!(query.conversation.is_none_or(|id| !id.is_empty()), "empty conversation");
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(&SQL[usize::from(query.conversation.is_some())])?;
        let candidates = statement.query_map(params![query.person,query.agent,query.through,
            query.before.unwrap_or(i64::MAX as u64),query.conversation,query.limit + 1], |row| {
            Ok((Candidate { message_id:row.get(0)?,claim_id:row.get(1)?,author:row.get(2)?,
                agent:row.get(3)?,conversation:row.get(4)?,accepted_order:row.get(5)? }, row.get::<_,bool>(6)?))
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        let has_more_candidates = candidates.len() > query.limit;
        let mut page = Page { entries: Vec::new(), examined_through: None, has_more_candidates };
        for (candidate, current) in candidates.into_iter().take(query.limit) {
            page.examined_through = Some(candidate.accepted_order);
            if current && authorized(&candidate)? { page.entries.push(candidate); }
        }
        Ok(page)
    }
}

#[cfg(test)]
mod tests;
