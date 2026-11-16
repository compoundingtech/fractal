//! Declared objectives (docs/st3/objectives.md): each `objective/NAME` declaration, and the member
//! an objective's owner is declared on.

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
}
