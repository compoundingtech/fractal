//! Which claim cites which, for a member that trims locally. A cut may drop a claim only if no
//! claim it stages cites it, and citations live in claim bodies (evidence, inputs, review
//! references, lineage) with no index. Reading every body each day costs what a full capture
//! does, so each cut first catches this cache up with the claims that arrived since the last
//! (a store-index cursor, in arrival order, so a citer that arrives long after what it cites
//! is still read), then looks its candidates up here.
//!
//! A row is a hint, not a fact: a cut counts a citer only if it is still present and sealed at
//! the cut, so a row for a claim another binary deleted keeps nothing alive. Deleting a claim
//! here removes its rows in the same transaction. A cursor that is missing, unreadable, ahead
//! of the store, or written under other checkpoint rules is not trusted: the cache is rebuilt.
use super::*;

pub(super) const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h2747359aff8a owner=smallclaims::store::local_trim_citations TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS local_trim_citations(
    cited TEXT NOT NULL, citer TEXT NOT NULL, PRIMARY KEY(cited, citer)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_trim_citations_citer ON local_trim_citations(citer);";

/// `meta`: the newest store index read, and the rules it was read under.
pub const CURSOR: &str = "local_trim_citations_read";

/// Claims read per catch-up page.
const PAGE: i64 = 256;

impl Store {
    /// Read every claim that arrived since the cache was last caught up and record what it
    /// cites. Each page's rows and the cursor commit in one transaction, so a catch-up cut
    /// short leaves a consistent prefix that the next one goes on from.
    pub fn catch_up_local_trim_citations(&self) -> Result<()> {
        let rules = self.runtime.checkpoint_rules_digest();
        let (saved, newest): (Option<String>, i64) = {
            let connection = self.readers.get();
            (
                connection.query_row("SELECT value FROM meta WHERE key=?1", [CURSOR], |row| row.get(0)).optional()?,
                connection.query_row("SELECT COALESCE(MAX(store_index), 0) FROM claims", [], |row| row.get(0))?,
            )
        };
        let trusted = saved
            .as_deref()
            .and_then(|saved| saved.split_once(' '))
            .filter(|(_, digest)| *digest == rules)
            .and_then(|(read, _)| read.parse::<i64>().ok())
            .filter(|read| (0..=newest).contains(read));
        let mut read = match trusted {
            Some(read) => read,
            None => {
                let mut connection = self.connection.write();
                let transaction = connection.transaction()?;
                transaction.execute(/* sqlite-large-value: allow-for-now lv-hd67b5a939818 owner=smallclaims::store::local_trim_citations TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_trim_citations", [])?;
                transaction.execute(
                    /* sqlite-large-value: allow-for-now lv-h6a249ef6d339 owner=smallclaims::store::local_trim_citations TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
                    params![CURSOR, format!("0 {rules}")],
                )?;
                transaction.commit()?;
                0
            }
        };
        let sql = format!("SELECT {CLAIM_COLUMNS} FROM claims WHERE store_index>?1 ORDER BY store_index LIMIT ?2");
        let mut pages = 0;
        while read < newest {
            let claims = self
                .readers
                .get()
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-h469f1627db34 owner=smallclaims::store::local_trim_citations TODO=trace-statement-and-value-representation */ &sql)?
                .query_map(params![read, PAGE], claim_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let Some(last) = claims.last().map(|claim| i64::try_from(claim.store_index)) else {
                break;
            };
            let last = last?;
            let rows: Vec<(String, String)> = claims
                .iter()
                .flat_map(|claim| {
                    self.runtime
                        .checkpoint_claim_facts(claim)
                        .cites
                        .into_iter()
                        .map(|cited| (cited, claim.id.clone()))
                })
                .collect();
            let mut connection = self.connection.write_background();
            let transaction = connection.transaction()?;
            {
                let mut insert = transaction
                    .prepare_cached(/* sqlite-large-value: allow-for-now lv-h7eb4aef1e4c3 owner=smallclaims::store::local_trim_citations TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_trim_citations(cited, citer) VALUES (?1, ?2)")?;
                for (cited, citer) in &rows {
                    insert.execute(/* sqlite-large-value: allow-for-now lv-h88167b4eff82 owner=smallclaims::store::local_trim_citations TODO=trace-statement-and-value-representation */ params![cited, citer])?;
                }
            }
            transaction.execute(
                /* sqlite-large-value: allow-for-now lv-h6b4470fbd115 owner=smallclaims::store::local_trim_citations TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
                params![CURSOR, format!("{last} {rules}")],
            )?;
            transaction.commit()?;
            drop(connection);
            read = last;
            pages += 1;
            self.stop_for_trim_fault(super::checkpoint_trim::TrimFault::AfterCitationPages(pages))?;
        }
        Ok(())
    }

    /// The claims citing `cited`, as the cache knows them.
    pub(crate) fn local_trim_citers(connection: &Connection, cited: &str) -> Result<Vec<String>> {
        Ok(connection
            .prepare_cached("SELECT citer FROM local_trim_citations WHERE cited=?1")?
            .query_map([cited], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Every citation a full read of the store finds, against the cache's, both between claims
    /// still present (a deleted claim keeps nothing, and its rows go with it): equal sets, so
    /// equal kept sets, are what `catch_up_local_trim_citations` promises. A bench and tests
    /// verify with it; it reads every claim.
    pub fn verify_local_trim_citations(&self) -> Result<LocalTrimCitationCheck> {
        self.catch_up_local_trim_citations()?;
        let connection = self.readers.get();
        let mut scanned = std::collections::BTreeSet::new();
        let sql = format!("SELECT {CLAIM_COLUMNS} FROM claims WHERE store_index>?1 ORDER BY store_index LIMIT ?2");
        let mut after = 0_i64;
        let mut claims_read = 0;
        loop {
            let claims = connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-hc4fe19b4483d owner=smallclaims::store::local_trim_citations TODO=trace-statement-and-value-representation */ &sql)?
                .query_map(params![after, PAGE], claim_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let Some(last) = claims.last() else { break };
            after = i64::try_from(last.store_index)?;
            claims_read += claims.len();
            let mut present = connection.prepare_cached("SELECT EXISTS(SELECT 1 FROM claims WHERE id=?1)")?;
            for claim in &claims {
                for cited in self.runtime.checkpoint_claim_facts(claim).cites {
                    if present.query_row(/* sqlite-large-value: allow-for-now lv-h988794483e9c owner=smallclaims::store::local_trim_citations TODO=trace-statement-and-value-representation */ [&cited], |row| row.get::<_, bool>(0))? {
                        scanned.insert((cited, claim.id.clone()));
                    }
                }
            }
        }
        let cached = connection
            .prepare(
                "SELECT cited, citer FROM local_trim_citations
                 WHERE EXISTS(SELECT 1 FROM claims WHERE claims.id=local_trim_citations.citer)
                   AND EXISTS(SELECT 1 FROM claims WHERE claims.id=local_trim_citations.cited)",
            )?
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()?;
        Ok(LocalTrimCitationCheck {
            claims_read,
            citations: scanned.len(),
            missing: scanned.difference(&cached).cloned().collect(),
            extra: cached.difference(&scanned).cloned().collect(),
        })
    }
}

/// What `verify_local_trim_citations` found.
#[derive(Debug)]
pub struct LocalTrimCitationCheck {
    pub claims_read: usize,
    pub citations: usize,
    /// Citations a full read finds that the cache lacks.
    pub missing: Vec<(String, String)>,
    /// Citations the cache holds for present claims that a full read does not find.
    pub extra: Vec<(String, String)>,
}
