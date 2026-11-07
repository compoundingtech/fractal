//! Temporary writer fence for the rolling upgrade that introduces private topic records.

use super::*;

impl Store {
    /// Set once, at daemon startup, after the operator attests every reader's privacy build.
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
                "SELECT json_extract(body,'$.fields.features.thread_read_privacy')=1 FROM claims
             WHERE subject=?1 AND kind='daemon.started' AND origin=?2
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
            ))
            .map_err(internal)?;
        for reader in readers {
            let supported: Option<bool> = feature
                .query_row(params![format!("daemon/{reader}"), reader], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(internal)?
                .flatten();
            if supported != Some(true) {
                return Err(unavailable());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_require_startup_attestation_and_all_configured_readers() {
        let writer = Store::open_memory("writer").unwrap();
        let reader = Store::open_memory("reader").unwrap();
        let check = || writer.thread_write_rollout(&writer.graph.readers.get());
        assert_eq!(check().unwrap_err().code, "thread-rollout-unavailable");
        writer.configure_thread_write_rollout(["reader".into()]);
        assert_eq!(check().unwrap_err().code, "thread-rollout-unavailable");
        reader
            .append_claim(&ClaimInput {
                subject: "daemon/reader".into(),
                kind: "daemon.started".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("features".into(), json!({"thread_read_privacy": 1})),
                ]),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        writer
            .import_replication("reader", &reader.export_replication(0).unwrap())
            .unwrap();
        // After the attested reader has advertised its floor, this check can pass offline.
        assert!(check().is_ok());
    }
}
