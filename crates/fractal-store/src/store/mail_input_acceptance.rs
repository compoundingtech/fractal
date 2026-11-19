//! Native input acceptance is not a generic delivered/read lifecycle assertion.
//! These owner-fenced receipts survive message/mark retention and retain the first acceptance.
use rusqlite::{Connection, params};

pub fn create_schema(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h5c91bcafc6ab owner=fractal_store::store::mail_input_acceptance TODO=chunk-or-reference-variable-columns */
        "CREATE TABLE IF NOT EXISTS local_mail_input_acceptance_v1 (
            message TEXT NOT NULL,
            recipient TEXT NOT NULL,
            incarnation TEXT NOT NULL,
            accepted_at_unix_ms INTEGER NOT NULL CHECK(accepted_at_unix_ms >= 0),
            recorded_at_unix_ms INTEGER NOT NULL,
            PRIMARY KEY(message, recipient, incarnation)
        );
        CREATE TABLE IF NOT EXISTS local_mail_acceptance_frontier_v1 (
            recipient TEXT NOT NULL,
            incarnation TEXT NOT NULL,
            observed_through_unix_ms INTEGER NOT NULL CHECK(observed_through_unix_ms >= 0),
            recorded_at_unix_ms INTEGER NOT NULL,
            PRIMARY KEY(recipient, incarnation)
        );",
    )?;
    Ok(())
}

/// The caller holds the native physical authority and checks the current mailbox fence in
/// this same transaction. Retrying a receipt cannot move the first observed acceptance.
pub fn record(
    connection: &Connection,
    message: &str,
    recipient: &str,
    incarnation: &str,
    accepted_at_unix_ms: u64,
    recorded_at_unix_ms: u64,
) -> anyhow::Result<bool> {
    anyhow::ensure!(accepted_at_unix_ms <= recorded_at_unix_ms, "input acceptance is in the future");
    Ok(connection.execute(
        /* sqlite-large-value: allow-for-now lv-h5c7f438f9d8a owner=fractal_store::store::mail_input_acceptance TODO=bound-or-reference-values-and-reclamation */
        "INSERT INTO local_mail_input_acceptance_v1
         (message,recipient,incarnation,accepted_at_unix_ms,recorded_at_unix_ms)
         VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(message,recipient,incarnation) DO UPDATE SET
             accepted_at_unix_ms=MIN(accepted_at_unix_ms,excluded.accepted_at_unix_ms)
         WHERE excluded.accepted_at_unix_ms < accepted_at_unix_ms",
        params![message, recipient, incarnation,
            i64::try_from(accepted_at_unix_ms)?, i64::try_from(recorded_at_unix_ms)?],
    )? != 0)
}

/// A source frontier is published only after the native producer sealed its source cut and
/// every acceptance through that cut committed. It is not the API receipt pump's wall clock.
pub fn record_frontier(
    connection: &Connection, recipient: &str, incarnation: &str,
    observed_through_unix_ms: u64, recorded_at_unix_ms: u64,
) -> anyhow::Result<bool> {
    anyhow::ensure!(observed_through_unix_ms <= recorded_at_unix_ms, "acceptance frontier is in the future");
    Ok(connection.execute(
        /* sqlite-large-value: allow-for-now lv-h61e1311c0afb owner=fractal_store::store::mail_input_acceptance TODO=bound-or-reference-values-and-reclamation */
        "INSERT INTO local_mail_acceptance_frontier_v1
         (recipient,incarnation,observed_through_unix_ms,recorded_at_unix_ms) VALUES(?1,?2,?3,?4)
         ON CONFLICT(recipient,incarnation) DO UPDATE SET
             observed_through_unix_ms=excluded.observed_through_unix_ms,
             recorded_at_unix_ms=excluded.recorded_at_unix_ms
         WHERE excluded.observed_through_unix_ms > observed_through_unix_ms",
        params![recipient, incarnation, i64::try_from(observed_through_unix_ms)?, i64::try_from(recorded_at_unix_ms)?],
    )? != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_acceptance_is_idempotent_and_incarnation_scoped() {
        let connection = Connection::open_in_memory().unwrap();
        create_schema(&connection).unwrap();
        assert!(record(&connection, "message/example", "agent/example", "one", 100, 110).unwrap());
        assert!(!record(&connection, "message/example", "agent/example", "one", 120, 130).unwrap());
        assert!(record(&connection, "message/example", "agent/example", "one", 90, 140).unwrap());
        assert!(record(&connection, "message/example", "agent/example", "two", 150, 160).unwrap());
        let times = connection.prepare("SELECT accepted_at_unix_ms FROM local_mail_input_acceptance_v1 ORDER BY incarnation")
            .unwrap().query_map([], |row| row.get::<_, u64>(0)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(times, [90, 150]);
        assert!(record(&connection, "message/future", "agent/example", "one", 170, 160).is_err());
    }

    #[test]
    fn frontiers_advance_monotonically_but_never_cross_recipient_incarnations() {
        let connection = Connection::open_in_memory().unwrap();
        create_schema(&connection).unwrap();
        assert!(record_frontier(&connection, "agent/example", "one", 100, 110).unwrap());
        assert!(!record_frontier(&connection, "agent/example", "one", 90, 120).unwrap());
        assert!(record_frontier(&connection, "agent/example", "one", 120, 130).unwrap());
        assert!(record_frontier(&connection, "agent/example", "two", 10, 20).unwrap());
        assert!(record_frontier(&connection, "agent/example", "two", 150, 140).is_err());
        let cuts = connection.prepare("SELECT observed_through_unix_ms FROM local_mail_acceptance_frontier_v1 ORDER BY incarnation")
            .unwrap().query_map([], |row| row.get::<_, u64>(0)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(cuts, [120, 10]);
    }
}
