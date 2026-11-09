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
            .prepare_cached(/* sqlite-large-value: allow-for-now lv-had7a23ff1f42 owner=fractal_store::store::thread_rollout TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT json_extract(body,'$.fields.features.thread_read_privacy') FROM claims
             WHERE subject=?1 AND kind='daemon.started' AND origin=?2
             ORDER BY CANONICAL_DESC(claims) LIMIT 1",
            ))
            .map_err(internal)?;
        for reader in readers {
            let privacy_version: Option<u32> = feature
                .query_row(/* sqlite-large-value: allow-for-now lv-h9aa2a0e11df4 owner=fractal_store::store::thread_rollout TODO=trace-statement-and-value-representation */ params![format!("daemon/{reader}"), reader], |row| {
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
        assert_eq!(check().unwrap_err().code, "thread-rollout-unavailable",
            "message privacy alone does not supply causal metadata reads");
        advertise(&reader, fractal_schema::threads::READ_PRIVACY_VERSION);
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

        assert!(!writer.initialize_thread_write_rollout(2, []));
        assert_eq!(check().unwrap_err().code, "thread-rollout-unavailable");
        assert!(writer.initialize_thread_write_rollout(fractal_schema::threads::READ_PRIVACY_VERSION, []));
        assert!(check().is_ok());
    }

    #[test]
    fn storage_23_upgrade_preserves_existing_history_and_pairing_admission() {
        for previous in [20_u32, 21, 22] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("claims.sqlite3");
        let store = Store::open(&path, "upgrade").unwrap();
        let person = "person/owner";
        let message = store.append_claim(&ClaimInput {
            subject:"message/before-upgrade".into(),kind:"message.sent".into(),actor:Some(person.into()),
            fields:serde_json::from_value(json!({"from":person,"to":"agent/reviewer",
                "content":"Retained history","status":"sent"})).unwrap(),
            evidence:vec![],expected_subject:None,idempotency_key:Some("retained-message".into()),
        }).unwrap();
        let paired = store.append_client_operation_claim(&ClaimInput {
            subject:"custom/client/pairing-before-upgrade".into(),kind:"custom.client.pairing-completed".into(),
            actor:Some(person.into()),fields:serde_json::from_value(json!({
                "person_id":person,"session_actor":"person/owner/session/phone","device_id":"device/phone",
                "credential_hash":"test-not-a-secret","expires_at_unix_ms":now_ms()+60_000,
                "scopes":["read.projections"],
            })).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:None,
        }).unwrap();
        assert!(store.pairing_completion_is_trusted(&paired, now_ms()).unwrap());
        drop(store);
        // The logical schema and stored claim formats are unchanged by this release.
        // Exercise the normal accepted open range, including published main's version 20.
        // This does not manufacture trust for an ambiguous old pairing record.
        Connection::open(&path).unwrap().pragma_update(None,"user_version",previous).unwrap();
        let upgraded = Store::open(&path,"upgrade").unwrap();
        assert_eq!(upgraded.readers.get().pragma_query_value(None,"user_version",|row|row.get::<_,u32>(0)).unwrap(),23);
        for original in [&message, &paired] {
            let retained = upgraded.graph.claim_by_id(&original.id).unwrap().unwrap();
            assert_eq!(retained.body, original.body);
            assert_eq!(retained.predecessors, original.predecessors);
        }
        assert_eq!(upgraded.operation_claim("retained-message").unwrap().unwrap().id,message.id);
        assert!(upgraded.pairing_completion_is_trusted(&paired,now_ms()).unwrap());
        assert!(!upgraded.pairing_completion_is_revoked(&paired).unwrap());
        }
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
