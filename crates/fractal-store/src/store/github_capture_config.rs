//! Source configurations are normal native desired state, queried by primary-key prefix.
use super::*;
use fractal_schema::github_capture::{SourceDefinition, declarations::parse_source};

impl Store {
    pub fn github_source_definitions(&self) -> Result<Vec<SourceDefinition>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached("SELECT body FROM desired WHERE subject>='github-source/' AND subject<'github-source0' AND kind='github-source' ORDER BY subject LIMIT 129")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut definitions = Vec::new();
        for row in rows {
            anyhow::ensure!(definitions.len() < 128, "GitHub source declaration read exceeds its bounded population");
            definitions.push(parse_source(&serde_json::from_str(&row?)?).map_err(anyhow::Error::msg)?);
        }
        Ok(definitions)
    }
}
