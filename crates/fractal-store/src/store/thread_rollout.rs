//! Temporary writer fence for the rolling upgrade that introduces private topic records.

use super::*;

impl Store {
    /// Apply the daemon startup opt-in only after this build protects the thread read surface.
    pub fn initialize_thread_write_rollout(
        &self,
        local_reader_version: u32,
        peers: impl IntoIterator<Item = String>,
    ) -> bool {
        if !fractal_schema::threads::read_privacy_supports_writes(local_reader_version) {
            return false;
        }
        self.configure_thread_write_rollout(peers);
        true
    }

    /// Set once after the local reader floor and operator attestation are confirmed.
    pub fn configure_thread_write_rollout(&self, peers: impl IntoIterator<Item = String>) {
        let _ = self.thread_write_peers.set(peers.into_iter().collect());
    }

    pub fn thread_write_rollout(&self, connection: &Connection) -> Result<(), FractalError> {
        let unavailable = || {
            FractalError::new(
                "thread-rollout-unavailable",
                "thread writes require the verified fleet reader rollout",
            )
        };
        let configured = self.thread_write_peers.get().ok_or_else(unavailable)?;
        let membership = fleet_membership_tx(connection).map_err(internal)?;
        let mut readers = configured.clone();
        readers.extend(
            membership
                .incarnations()
                .filter(|member| member.end.is_none())
                .map(|member| member.name.clone()),
        );
        readers.remove(self.origin());
        let mut feature = connection
            .prepare_cached(&canonical_sql(
                "SELECT json_extract(body,'$.fields.features.thread_read_privacy') FROM claims
             WHERE subject=?1 AND kind='daemon.started' AND origin=?2
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
            ))
            .map_err(internal)?;
        for reader in readers {
            let privacy_version: Option<u32> = feature
                .query_row(params![format!("daemon/{reader}"), reader], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(internal)?
                .flatten();
            if !fractal_schema::threads::read_privacy_supports_writes(
                privacy_version.unwrap_or_default(),
            ) {
                return Err(unavailable());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advertise(reader: &Store, version: u32) {
        reader
            .append_claim(&ClaimInput {
                subject: format!("daemon/{}", reader.origin()),
                kind: "daemon.started".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("features".into(), json!({"thread_read_privacy": version})),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }

    #[test]
    fn writes_require_startup_attestation_and_all_configured_readers() {
        let writer = Store::open_memory("writer").unwrap();
        let reader = Store::open_memory("reader").unwrap();
        let check = || writer.thread_write_rollout(&writer.graph.readers.get());
        assert_eq!(check().unwrap_err().code, "thread-rollout-unavailable");
        writer.configure_thread_write_rollout(["reader".into()]);
        assert_eq!(check().unwrap_err().code, "thread-rollout-unavailable");
        advertise(&reader, 2);
        writer
            .import_replication("reader", &reader.export_replication(0).unwrap())
            .unwrap();
        // After the attested reader has advertised its floor, this check can pass offline.
        assert!(check().is_ok());
    }

    #[test]
    fn singleton_startup_requires_complete_local_read_privacy() {
        let writer = Store::open_memory("writer").unwrap();
        let check = || writer.thread_write_rollout(&writer.graph.readers.get());

        assert!(!writer.initialize_thread_write_rollout(1, []));
        assert_eq!(check().unwrap_err().code, "thread-rollout-unavailable");

        assert!(writer.initialize_thread_write_rollout(2, []));
        assert!(check().is_ok());
    }

    #[test]
    fn generic_read_privacy_does_not_unlock_thread_writes() {
        let writer = Store::open_memory("writer").unwrap();
        let reader = Store::open_memory("legacy-reader").unwrap();
        writer.configure_thread_write_rollout([reader.origin().to_owned()]);
        advertise(&reader, 1);
        writer
            .import_replication(reader.origin(), &reader.export_replication(0).unwrap())
            .unwrap();
        let error = writer
            .thread_write_rollout(&writer.graph.readers.get())
            .unwrap_err();
        assert_eq!(error.code, "thread-rollout-unavailable");
    }
}
