//! Complete current runtime resources, published off the HTTP request path.
use super::*;

pub struct RuntimeRows {
    pub items: Arc<Vec<Value>>,
    /// Computed once by the refresher, never by a page read.
    pub digest: String,
    pub time_unix_ms: u128,
}

impl Store {
    pub fn published_runtime_list(&self) -> &published_list::PublishedList<RuntimeRows> {
        &self.fractal.published_runtimes
    }

    /// Runtime rows read all claims on their subject (including unknown and reserved
    /// kinds), their declaration, and the actual state of its owning run/generation.
    /// Use the same bounded commit scan as the other published lists, then seek only
    /// the subjects that have runtime observations.
    pub fn runtime_list_changes(&self, after: u64, through: u64) -> Result<BTreeSet<String>> {
        let connection = self.readers.get();
        let mut changed = connection.prepare_cached(published_list::CLAIMS_SINCE)?
            .query_map(params![after, through, Option::<&str>::None], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        // A conflicting operation changes the authority of its earlier subjects too,
        // even when the new claim names an unrelated subject.
        for row in connection.prepare_cached(
            "SELECT prior.subject FROM claims incoming CROSS JOIN claims prior
             INDEXED BY claims_operation_index
             ON json_extract(prior.body,'$._operation.id')=json_extract(incoming.body,'$._operation.id')
             WHERE incoming.store_index>?1 AND incoming.store_index<=?2
               AND json_extract(incoming.body,'$._operation.id') IS NOT NULL
               AND prior.store_index<=?2",
        )?.query_map(params![after, through], |row| row.get::<_, String>(0))? {
            changed.insert(row?);
        }
        let mut sets = BTreeSet::new();
        for row in connection.prepare_cached(
            "SELECT CASE WHEN kind='owned-set.revised' THEN subject
                    ELSE json_extract(body,'$.owned_set') END FROM claims
             WHERE store_index>?1 AND store_index<=?2
               AND (kind='owned-set.revised' OR json_extract(body,'$.owned_set') IS NOT NULL)",
        )?.query_map(params![after, through], |row| row.get::<_, String>(0))? {
            sets.insert(row?);
        }
        for set in sets {
            // Include losing-branch members omitted by the winning receipt: they
            // receive implicit stop declarations without a new claim of their own.
            changed.extend(owned_sets::known_members(&connection, &set, Some(through))?.into_keys());
        }
        let mut candidates = BTreeSet::new();
        for subject in &changed {
            let query = if subject.starts_with("mission-run/") {
                "SELECT subject FROM desired WHERE owner_run=?1"
            } else if subject.starts_with("run-generation/") {
                "SELECT subject FROM desired WHERE owner_generation=?1"
            } else {
                continue;
            };
            for row in connection.prepare_cached(query)?
                .query_map([subject], |row| row.get::<_, String>(0))? {
                candidates.insert(row?);
            }
        }
        candidates.extend(changed);
        let mut observed = connection.prepare_cached(
            "SELECT 1 FROM claims WHERE subject=?1 AND kind='runtime.observed'
             AND store_index<=?2 LIMIT 1",
        )?;
        let mut affected = BTreeSet::new();
        for subject in candidates {
            if observed.query_row(params![subject, through], |_| Ok(())).optional()?.is_some() {
                affected.insert(subject);
            }
        }
        Ok(affected)
    }
}
