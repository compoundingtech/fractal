//! Header-only seeks over accepted person sends. The claims log owns the bodies;
//! this projection adds no prompt journal and cannot query across agents.
use super::*;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h34f56ac109e6 owner=fractal_store::store::input_history TODO=chunk-or-reference-variable-columns */
        "CREATE INDEX IF NOT EXISTS input_history_author_agent ON claims(
         actor,json_extract(body,'$.fields.to'),printf('%039s',accepted_at_unix_ms) DESC,subject DESC,id DESC)
         WHERE kind='message.sent' AND json_extract(body,'$.fields.from')=actor;
         CREATE INDEX IF NOT EXISTS input_history_author_conversation ON claims(
         actor,json_extract(body,'$.fields.to'),json_extract(body,'$.fields.session_id'),
         printf('%039s',accepted_at_unix_ms) DESC,subject DESC,id DESC)
         WHERE kind='message.sent' AND json_extract(body,'$.fields.from')=actor;",
    )?;
    Ok(())
}
static SQL: std::sync::LazyLock<[String; 2]> = std::sync::LazyLock::new(|| {
    [("input_history_author_agent", "AND ?7 IS NULL"),
     ("input_history_author_conversation", "AND json_extract(claims.body,'$.fields.session_id')=?7")]
        .map(|(index, filter)| canonical_sql(&format!(
            "SELECT claims.subject,claims.id,claims.actor,
             json_extract(claims.body,'$.fields.to'),
             json_extract(claims.body,'$.fields.session_id'),printf('%039s',claims.accepted_at_unix_ms),
             ({UNREPAIRED_CLAIM} AND claims.store_index<=?3 AND claims.id=(
                SELECT sent.id FROM claims AS sent INDEXED BY claims_subject_kind_accepted_index
                WHERE sent.subject=claims.subject AND sent.kind='message.sent'
                ORDER BY CANONICAL_ASC(sent) LIMIT 1))
             FROM claims INDEXED BY {index}
             WHERE claims.kind='message.sent' AND claims.actor=?1
             AND json_extract(claims.body,'$.fields.to')=?2
             AND json_extract(claims.body,'$.fields.from')=claims.actor
             AND (printf('%039s',claims.accepted_at_unix_ms),claims.subject,claims.id)<(?4,?5,?6)
             {filter} ORDER BY printf('%039s',claims.accepted_at_unix_ms) DESC,
             claims.subject DESC,claims.id DESC LIMIT ?8")))
});


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub message_id: String,
    pub claim_id: String,
    pub author: String,
    pub agent: String,
    pub conversation: Option<String>,
    pub accepted_order: String,
}

/// The extra claim ID advances past noncanonical duplicate source rows.
#[derive(Clone, Debug)]
pub struct Cursor {
    pub accepted_order: String,
    pub message_id: String,
    pub claim_id: String,
}

/// `through` freezes insert membership; current repairs/deletions still win.
pub struct Query<'a> {
    pub person: &'a str,
    pub agent: &'a str,
    pub conversation: Option<&'a str>,
    pub through: u64,
    pub before: Option<&'a Cursor>,
    pub limit: usize,
}

/// A bounded candidate window, not a count of visible history or a text search.
/// Continue after `examined_through`, even if authorization removed every row.
#[derive(Debug)]
pub struct Page {
    pub entries: Vec<Candidate>,
    pub examined_through: Option<Cursor>,
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
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h84e689b7b7df owner=fractal_store::store::input_history TODO=trace-statement-and-value-representation */ &SQL[usize::from(query.conversation.is_some())])?;
        let candidates = statement.query_map(params![query.person,query.agent,query.through,
            query.before.map_or("\u{10ffff}", |c|c.accepted_order.as_str()),
            query.before.map_or("", |c|c.message_id.as_str()),
            query.before.map_or("", |c|c.claim_id.as_str()),query.conversation,query.limit + 1], |row| {
            Ok((Candidate { message_id:row.get(0)?,claim_id:row.get(1)?,author:row.get(2)?,
                agent:row.get(3)?,conversation:row.get(4)?,accepted_order:row.get(5)? }, row.get::<_,bool>(6)?))
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        let has_more_candidates = candidates.len() > query.limit;
        let examined_through = candidates.iter().take(query.limit).last().map(|(c,_)| Cursor {
            accepted_order:c.accepted_order.clone(),message_id:c.message_id.clone(),claim_id:c.claim_id.clone()
        });
        let mut page = Page { entries: Vec::new(), examined_through, has_more_candidates };
        for (candidate, current) in candidates.into_iter().take(query.limit) {
            if current && authorized(&candidate)? { page.entries.push(candidate); }
        }
        Ok(page)
    }
}

#[cfg(test)]
mod tests;
