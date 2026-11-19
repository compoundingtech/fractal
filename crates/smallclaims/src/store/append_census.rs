//! Connection-lifetime append accounting. TEMP state follows transaction rollback and never
//! reads retained claim bodies. Bytes mean UTF-8 claim-body bytes, not SQLite amplification.
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AppendCounter {
    pub kind: String,
    pub claims: u64,
    pub body_bytes: u64,
}

pub(super) fn install(connection: &Connection) -> anyhow::Result<()> {
    let existing: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='claims_stored')", [], |row| row.get(0),
    )?;
    if existing {
        // Reuse the graph's existing write-time metrics; only copy their tiny baseline, not
        // claim history. This avoids adding a second counter write to every graph append.
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hf81b16e35691 owner=smallclaims::store::append_census TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE storage_append_baseline (
                 kind TEXT, writer TEXT, claims INTEGER, bytes INTEGER,
                 PRIMARY KEY(kind, writer)
             ) WITHOUT ROWID;
             INSERT INTO storage_append_baseline SELECT kind, writer, claims, bytes FROM main.claims_stored;
             CREATE TEMP VIEW storage_append_counters AS
             SELECT c.kind, sum(c.claims-coalesce(b.claims,0)) AS claims,
                    sum(c.bytes-coalesce(b.bytes,0)) AS body_bytes
             FROM main.claims_stored c LEFT JOIN storage_append_baseline b
               ON c.kind=b.kind AND c.writer=b.writer
             GROUP BY c.kind;"
        )?;
    } else {
        // The messages runtime has no exporter counters. Keep its counters connection-local,
        // with the same transactional body-byte accounting as the graph's claims_stored.
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hdb019a72e227 owner=smallclaims::store::append_census TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE storage_append_counters (
                 kind TEXT PRIMARY KEY, claims INTEGER NOT NULL, body_bytes INTEGER NOT NULL
             ) WITHOUT ROWID;
             CREATE TEMP TRIGGER storage_append AFTER INSERT ON main.claims BEGIN
                 INSERT INTO storage_append_counters VALUES
                     (NEW.kind, 1, length(CAST(NEW.body AS BLOB)))
                 ON CONFLICT(kind) DO UPDATE SET
                     claims=claims+1, body_bytes=body_bytes+excluded.body_bytes;
             END;"
        )?;
    }
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h8d638151f55e owner=smallclaims::store::append_census TODO=chunk-or-reference-variable-columns */ "CREATE TEMP TABLE storage_census_start(unix_ms INTEGER NOT NULL);")?;
    connection.execute(/* sqlite-large-value: allow-for-now lv-hae824bbd6681 owner=smallclaims::store::append_census TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO storage_census_start VALUES (?1)", [super::now_ms() as i64])?;
    Ok(())
}

impl super::Store {
    pub fn append_census_since(&self) -> anyhow::Result<u128> {
        Ok(self.connection.write().query_row(
            "SELECT unix_ms FROM temp.storage_census_start", [], |row| row.get::<_, i64>(0),
        )? as u128)
    }
    /// Reads only the write-time materialization, never the claims table. Includes local and
    /// replicated admission; retries which insert no row and rolled-back inserts count zero.
    pub fn append_census(&self) -> anyhow::Result<Vec<AppendCounter>> {
        let connection = self.connection.write();
        let mut statement = connection.prepare_cached(
            "SELECT kind, claims, body_bytes FROM temp.storage_append_counters ORDER BY kind"
        )?;
        Ok(statement.query_map([], |row| Ok(AppendCounter {
            kind: row.get(0)?, claims: row.get(1)?, body_bytes: row.get(2)?,
        }))?.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_census_counts_utf8_and_rolls_back_without_scanning_history() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE claims(kind TEXT, body TEXT);").unwrap();
        install(&connection).unwrap();
        connection.execute("INSERT INTO claims VALUES ('custom.test', ?1)", ["é"]).unwrap();
        connection.execute_batch("BEGIN; INSERT INTO claims VALUES ('custom.test', 'lost'); ROLLBACK;").unwrap();
        let counts: (u64, u64) = connection.query_row(
            "SELECT claims, body_bytes FROM storage_append_counters", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(counts, (1, 2));
        connection.execute("DELETE FROM claims", []).unwrap();
        assert_eq!(connection.query_row("SELECT claims FROM storage_append_counters", [], |row| row.get::<_, u64>(0)).unwrap(), 1);
    }
}
