//! Member-local APNs registrations. Secrets never enter claims or replication.
use super::*;

pub(crate) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS local_push_registrations (
            person TEXT NOT NULL, device_id TEXT NOT NULL, pairing_grant TEXT NOT NULL,
            pairing_index INTEGER NOT NULL, expires_at INTEGER NOT NULL,
            token TEXT NOT NULL, environment TEXT NOT NULL CHECK(environment IN ('development','production')),
            bundle_id TEXT NOT NULL, app_version TEXT NOT NULL,
            PRIMARY KEY(person,device_id)
         );
         CREATE INDEX IF NOT EXISTS local_push_registrations_grant ON local_push_registrations(pairing_grant);
         CREATE INDEX IF NOT EXISTS local_push_registrations_expiry ON local_push_registrations(expires_at);
         CREATE TRIGGER IF NOT EXISTS local_push_pairing_changed AFTER INSERT ON claims
         WHEN NEW.kind IN ('custom.client.pairing-completed','custom.client.pairing-revoked')
         BEGIN
            DELETE FROM local_push_registrations
            WHERE pairing_grant=NEW.subject AND pairing_index<NEW.store_index;
         END;",
    )?;
    Ok(())
}

/// A registration deliberately has no Debug or Serialize implementation.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PushRegistration {
    pub token: String,
    pub environment: PushEnvironment,
    pub bundle_id: String,
    pub app_version: String,
}

#[derive(Clone, Copy, Debug, serde::Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PushEnvironment {
    Development,
    Production,
}

impl PushEnvironment {
    fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct PushRegistrationStatus {
    pub registered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<PushEnvironment>,
}

impl PushRegistrationStatus {
    pub(crate) fn unregistered() -> Self {
        Self {
            registered: false,
            environment: None,
        }
    }
}

impl Store {
    /// Resolve device identity and recheck the exact grant under the same writer transaction
    /// as the mutation. A concurrent revocation cannot leave a newly registered secret behind.
    pub(crate) fn update_push_registration(
        &self,
        grant: &str,
        person: &str,
        actor: &str,
        registration: Option<&PushRegistration>,
        now: u64,
    ) -> Result<Option<PushRegistrationStatus>> {
        self.connection.batched(|tx| -> Result<Option<PushRegistrationStatus>> {
            let pairing: Option<(u64, String, u64)> = tx.prepare_cached(
                "SELECT store_index,json_extract(body,'$.fields.device_id'),json_extract(body,'$.fields.expires_at_unix_ms')
                 FROM claims WHERE subject=?1 AND kind='custom.client.pairing-completed'
                   AND origin=?2 AND json_extract(body,'$.fields.person_id')=?3
                   AND json_extract(body,'$.fields.session_actor')=?4
                   AND json_type(body,'$.fields.device_id')='text'
                   AND json_type(body,'$.fields.expires_at_unix_ms')='integer'
                 ORDER BY store_index DESC LIMIT 1",
            )?.query_row(params![grant, self.origin(), person, actor], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            }).optional()?;
            let Some((index, device, expiry)) = pairing else { return Ok(None); };
            let revoked: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1
                 AND kind IN ('custom.client.pairing-completed','custom.client.pairing-revoked') AND store_index>?2)",
                params![grant, index], |row| row.get(0),
            )?;
            if revoked || expiry <= now || device.is_empty() {
                tx.execute("DELETE FROM local_push_registrations WHERE pairing_grant=?1", [grant])?;
                return Ok(None);
            }
            if let Some(registration) = registration {
                tx.execute(
                    "INSERT INTO local_push_registrations(person,device_id,pairing_grant,pairing_index,expires_at,token,environment,bundle_id,app_version)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
                     ON CONFLICT(person,device_id) DO UPDATE SET pairing_grant=excluded.pairing_grant,
                     pairing_index=excluded.pairing_index,expires_at=excluded.expires_at,token=excluded.token,
                     environment=excluded.environment,bundle_id=excluded.bundle_id,app_version=excluded.app_version",
                    params![person, device, grant, index, expiry, registration.token,
                        registration.environment.as_str(), registration.bundle_id, registration.app_version],
                )?;
                Ok(Some(PushRegistrationStatus { registered: true, environment: Some(registration.environment) }))
            } else {
                tx.execute("DELETE FROM local_push_registrations WHERE person=?1 AND device_id=?2", params![person, device])?;
                Ok(Some(PushRegistrationStatus::unregistered()))
            }
        }).map_err(|_| anyhow::anyhow!("local push registration transaction failed"))?
            .map_err(|_| anyhow::anyhow!("local push registration update failed"))
    }

    /// Startup and clock-driven cleanup, independent of attention/window reads. The trigger
    /// handles local and replicated grant changes atomically with claim admission.
    pub(crate) fn sweep_push_registrations(&self, now: u64) -> Result<usize> {
        self.connection.batched(|tx| -> Result<usize> {
            Ok(tx.execute(
                "DELETE FROM local_push_registrations WHERE expires_at<=?1 OR NOT EXISTS(
                    SELECT 1 FROM claims WHERE subject=pairing_grant AND store_index=pairing_index
                    AND kind='custom.client.pairing-completed') OR EXISTS(
                    SELECT 1 FROM claims WHERE subject=pairing_grant AND store_index>pairing_index
                    AND kind IN ('custom.client.pairing-completed','custom.client.pairing-revoked'))", [now],
            )?)
        }).map_err(|_| anyhow::anyhow!("local push registration transaction failed"))?
            .map_err(|_| anyhow::anyhow!("local push registration cleanup failed"))
    }

    /// Select only nonsecret fields: resource/debug/error paths never load a token.
    pub(crate) fn push_registration_statuses(
        &self,
        person: &str,
        now: u64,
    ) -> Result<BTreeMap<String, PushRegistrationStatus>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT device_id,environment FROM local_push_registrations WHERE person=?1 AND expires_at>?2
             AND NOT EXISTS(SELECT 1 FROM claims WHERE subject=pairing_grant AND store_index>pairing_index
                 AND kind IN ('custom.client.pairing-completed','custom.client.pairing-revoked'))",
        )?;
        Ok(statement
            .query_map(params![person, now], |row| {
                let environment = match row.get_ref(1)?.as_str()? {
                    "development" => PushEnvironment::Development,
                    "production" => PushEnvironment::Production,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok((
                    row.get(0)?,
                    PushRegistrationStatus {
                        registered: true,
                        environment: Some(environment),
                    },
                ))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests;
