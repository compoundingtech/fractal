//! Declared objectives (docs/st3/objectives.md): each `objective/NAME` declaration, and the claims
//! an interval objective reads.

use super::*;
use crate::objective::Objective;

/// One current `objective/NAME` declaration. A body the kernel reader refuses is kept with its
/// error, so the objective's row says so instead of disappearing.
#[derive(Clone, Debug)]
pub struct DeclaredObjective {
    pub subject: String,
    pub revision: String,
    pub claim_id: String,
    pub objective: std::result::Result<Objective, String>,
}

impl DeclaredObjective {
    /// The objective's name: its subject after `objective/`.
    pub fn name(&self) -> &str {
        self.subject.strip_prefix("objective/").unwrap_or(&self.subject)
    }
}

pub fn declared_objectives_at(connection: &Connection) -> Result<Vec<DeclaredObjective>> {
    let mut statement = connection.prepare_cached(
        "SELECT subject, revision, claim_id, body FROM desired WHERE kind='objective' ORDER BY subject",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
    })?;
    let mut objectives = Vec::new();
    for row in rows {
        let (subject, revision, claim_id, body) = row?;
        let objective = serde_json::from_str::<Value>(&body)
            .map_err(|error| format!("its body is not JSON: {error}"))
            .and_then(|desired| crate::objective::parse(&desired).map_err(|error| format!("{}: {}", error.code, error.message)));
        objectives.push(DeclaredObjective { subject, revision, claim_id, objective });
    }
    Ok(objectives)
}

/// Claims of `kinds` (`?5`…) after store index `?1`, at most `?2` rows, under the subject range
/// `?3 <= subject < ?4` when `prefix`.
fn kinds_after_query(kinds: usize, prefix: bool) -> String {
    let placeholders = (0..kinds).map(|n| format!("?{}", n + 5)).collect::<Vec<_>>().join(",");
    let subject_filter = if prefix { " AND subject>=?3 AND subject<?4" } else { "" };
    format!(
        "SELECT id, store_index, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms
         FROM claims INDEXED BY claims_kind_index
         WHERE kind IN ({placeholders}) AND store_index>?1{subject_filter}
         ORDER BY store_index LIMIT ?2"
    )
}

impl Store {
    pub fn declared_objectives(&self) -> Result<Vec<DeclaredObjective>> {
        declared_objectives_at(&self.readers.get())
    }

    /// The member `agent` is declared on, when it is a current agent declaration.
    pub fn agent_host(&self, agent: &str) -> Result<Option<String>> {
        Ok(self
            .readers
            .get()
            .prepare_cached("SELECT json_extract(member,'$.host') FROM desired WHERE subject=?1 AND kind='agent'")?
            .query_row(/* sqlite-large-value: allow-for-now lv-hbb373c1f9535 owner=fractal_store::store::declared_objectives TODO=trace-statement-and-value-representation */ [agent], |row| row.get::<_, Option<String>>(0))
            .optional()?
            .flatten())
    }

    /// The next `limit` claims of any of `kinds` after `after_index`, ascending, optionally under
    /// `subject_prefix`. Each kind seeks `claims_kind_index` past the cursor, so a caller that keeps
    /// the cursor reads only claims newer than the last it saw.
    pub fn claims_by_kinds_after(
        &self,
        kinds: &[&str],
        subject_prefix: Option<&str>,
        after_index: u64,
        limit: usize,
    ) -> Result<ClaimsPage> {
        if kinds.is_empty() {
            return Ok(ClaimsPage { claims: Vec::new(), next_cursor: None });
        }
        let range = subject_prefix
            .map(|prefix| {
                prefix_upper_bound(prefix).map(|upper| (prefix, upper)).ok_or_else(|| {
                    anyhow::Error::from(FractalError::new(
                        "invalid-claims-query",
                        "a subject prefix must be non-empty ASCII without `*`, `?` or `[`",
                    ))
                })
            })
            .transpose()?;
        let query = kinds_after_query(kinds.len(), range.is_some());
        let scan_limit = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let (lower, upper) = range.map_or((None, None), |(lower, upper)| (Some(lower), Some(upper)));
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![&after_index, &scan_limit, &lower, &upper];
        values.extend(kinds.iter().map(|kind| kind as &dyn rusqlite::ToSql));
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h5073cff5e5e7 owner=fractal_store::store::declared_objectives TODO=trace-statement-and-value-representation */ &query)?;
        let mut claims = statement
            .query_map(rusqlite::params_from_iter(values), claim_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let next_cursor = if claims.len() > limit {
            claims.pop();
            claims.last().map(|claim| claim.store_index)
        } else {
            None
        };
        Ok(ClaimsPage { claims, next_cursor })
    }
}
