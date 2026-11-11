//! Durable live-projection refusal classification; not a new wire or storage schema.
use super::*;

pub const CODE: &str = "projection-recovery-required";
const PREFIX: &str = "projection-recovery-required/";

pub fn error_code(reason: &'static str) -> String {
    format!("{PREFIX}{reason}")
}

pub fn reason(code: &str) -> Option<&str> {
    if code == CODE {
        return Some("unknown");
    }
    code.strip_prefix(PREFIX).map(|reason| if reason.is_empty() { "unknown" } else { reason })
}

/// Missing health has no last-good proof, even when an old schema defaults its column to zero.
pub(super) fn last_good(frontier: Option<u64>, code: Option<&str>) -> Option<u64> {
    if code.and_then(reason) == Some("missing-health") { None } else { frontier }
}

/// One read snapshot, shared by health, doctor and the operator fault producer.
/// `current_target` is the latest durable admitted store index, not a failed attempt's target.
#[derive(Debug)]
pub struct RecoveryRequired {
    pub frontier: Option<u64>,
    pub reason_code: String,
    pub current_target: u64,
    pub message: Option<String>,
    pub updated_at_unix_ms: String,
}

pub(super) fn required(connection: &Connection) -> Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate='graph' AND status='stale'
           AND (error_code=?1 OR substr(error_code,1,length(?2))=?2))",
        params![CODE, PREFIX], |row| row.get(0),
    ).map_err(Into::into)
}

impl Store {
    pub fn projection_recovery(&self) -> Result<Option<RecoveryRequired>> {
        let connection = self.readers.try_get()?;
        connection.busy_timeout(std::time::Duration::ZERO)?;
        let row = connection.query_row(
            "SELECT last_good_store_index,error_code,
                    (SELECT value FROM meta WHERE key='replication_admitted_index'),
                    error_message,updated_at_unix_ms
             FROM projection_health WHERE aggregate='graph' AND status='stale'
               AND (error_code=?1 OR substr(error_code,1,length(?2))=?2)",
            params![CODE, PREFIX],
            |row| Ok((row.get::<_, Option<u64>>(0)?, row.get::<_, String>(1)?,
                row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, String>(4)?)),
        ).optional()?;
        row.map(|(frontier, code, target, message, updated_at_unix_ms)| {
            Ok(RecoveryRequired { frontier: last_good(frontier, Some(&code)),
                reason_code: reason(&code).unwrap_or("unknown").into(),
                current_target: target.parse()?, message, updated_at_unix_ms })
        }).transpose()
    }

    pub fn replication_projection_recovery_required(&self) -> Result<bool> {
        required(&self.readers.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_reason_round_trip_and_legacy_unknown() {
        assert_eq!(reason(&error_code("missing-health")), Some("missing-health"));
        assert_eq!(reason(CODE), Some("unknown"));
        assert_eq!(reason(PREFIX), Some("unknown"));
        assert_eq!(reason("pending-projection-work"), None);
        assert_eq!(reason("repair-failed"), None);
        assert_eq!(reason("projection-recovery-required-other"), None);
        assert_eq!(last_good(Some(0), Some(&error_code("missing-health"))), None);
        assert_eq!(last_good(Some(0), Some(&error_code("internal"))), Some(0));
        assert_eq!(last_good(None, Some(CODE)), None);
    }
}
