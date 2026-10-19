//! The part of `store::runtime` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
pub use st3_store::store::runtime::*;
pub use st3_store::store::runtime::{AgentResourcesEntry, AgentRosterFaultRetries, Extensions, SmalltalkRuntime, compatibility_digest};

#[cfg(test)]
mod schema_install_tests {
    use super::*;
    use st3_store::store::runtime_schema_install_tests_shared_attention_tests::{crash_during_schema_install_child};

    #[test]
    fn crashed_schema_chunks_recover_without_losing_existing_data() {
        if std::env::var_os("ST3_SCHEMA_CRASH_PHASE").is_some() {
            crash_during_schema_install_child();
            unreachable!("the private child must exit at its crash point");
        }
        for phase in ["core", "runtime", "late-runtime"] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("schema.sqlite");
            let marker = directory.path().join("crash-marker");
            {
                let connection = Connection::open(&path).unwrap();
                connection.execute_batch("CREATE TABLE fixture_marker(value TEXT); INSERT INTO fixture_marker VALUES('preserved'); PRAGMA user_version=18;").unwrap();
                if phase != "core" {
                    connection
                        .execute_batch(smallclaims::store::SCHEMA)
                        .unwrap();
                }
            }
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "store::runtime::schema_install_tests::crashed_schema_chunks_recover_without_losing_existing_data",
                    "--nocapture",
                ])
                .env("ST3_SCHEMA_CRASH_PATH", &path)
                .env("ST3_SCHEMA_CRASH_MARKER", &marker)
                .env("ST3_SCHEMA_CRASH_PHASE", phase)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86), "{phase}: crash hook must run");
            assert_eq!(std::fs::read_to_string(&marker).unwrap(), phase);
            {
                let connection = Connection::open(&path).unwrap();
                let partial = if phase == "core" {
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name IN ('meta','batches'))"
                } else if phase == "runtime" {
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='claims_owned_set_subject_index')"
                } else {
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='resource_observations')"
                };
                assert!(
                    !connection
                        .query_row(partial, [], |row| row.get::<_, bool>(0))
                        .unwrap(),
                    "{phase}: uncommitted DDL survived the process crash"
                );
            }
            let recovered = Store::open(&path, "birch").unwrap();
            let reader = recovered.readers.get();
            assert_eq!(
                reader
                    .query_row("SELECT value FROM fixture_marker", [], |row| row
                        .get::<_, String>(0))
                    .unwrap(),
                "preserved"
            );
            assert_eq!(
                reader
                    .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                    .unwrap(),
                st3_schema::STORAGE_VERSION
            );
            assert_eq!(
                recovered
                    .connection
                    .write()
                    .pragma_query_value(None, "synchronous", |row| row.get::<_, u32>(0))
                    .unwrap(),
                2
            );
        }
    }
}
