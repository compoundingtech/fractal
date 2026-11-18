//! Remove historical captured shell environments before the store serves requests.
//!
//! This is logical deletion, not secure erasure: free pages, WAL, backups, and
//! replicas that have not started an updated binary can retain historical bytes.
//! Credential rotation is independent and must not wait for this scrub. Shell
//! custody is owner-local and is never imported by claims replication.
use super::*;

/// Idempotent on every startup. Only a row count leaves this operation.
pub(super) fn scrub(connection: &Connection) -> Result<usize> {
    Ok(connection.execute(
        "UPDATE shell_resources_v1 SET body=json_remove(body,'$.command.environment')
         WHERE json_type(body,'$.command.environment') IS NOT NULL",
        [],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn historical(connection: &Connection) {
        connection.execute(
            "INSERT INTO shell_resources_v1(id,host,state,body) VALUES('old','example','queued',?1)",
            [r#"{"command":{"environment":{"PRIVATE_TOKEN":"private-environment-value"}},"state":"queued"}"#],
        ).unwrap();
    }

    #[test]
    fn environment_scrub_is_idempotent_and_counts_only() {
        let store = Store::open_memory("example").unwrap();
        let connection = store.connection.write();
        historical(&connection);
        assert_eq!(scrub(&connection).unwrap(), 1);
        assert_eq!(scrub(&connection).unwrap(), 0);
        let body: String = connection.query_row("SELECT body FROM shell_resources_v1 WHERE id='old'", [], |row| row.get(0)).unwrap();
        assert!(!body.contains("private-environment-value"));
        assert!(!body.contains("PRIVATE_TOKEN"));
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["state"], "queued");
    }

    #[test]
    fn environment_scrub_runs_when_store_reopens() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.sqlite3");
        {
            let store = Store::open(&path, "example").unwrap();
            historical(&store.connection.write());
        }
        for _ in 0..2 {
            let store = Store::open(&path, "example").unwrap();
            let body: String = store.readers.get().query_row(
                "SELECT body FROM shell_resources_v1 WHERE id='old'", [], |row| row.get(0),
            ).unwrap();
            assert!(!body.contains("private-environment-value"));
            assert!(!body.contains("environment"));
        }
    }
}
