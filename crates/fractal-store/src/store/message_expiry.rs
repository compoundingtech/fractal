//! Durable product outcomes, separate from member-local transport receipts. No expiry writer
//! runs here. Readers retain expiry history even when later consumption evidence supersedes it.
use super::*;

pub const KIND: &str = "message.expired";

/// Cross-file ordering: take this before either writer and retain it through commit.
#[derive(Default)]
pub(super) struct ReceiptGate(pub Mutex<()>);

#[derive(Clone, Debug)]
pub struct MessageExpiry {
    pub claim: ClaimRecord,
    /// Actual delivery/read evidence wins; expiry never asserts that consumption was impossible.
    pub superseded: bool,
}

impl Store {
    pub fn message_expiry(&self, subject: &str) -> Result<Option<MessageExpiry>> {
        let Some(claim) = self.latest_claim(subject, Some(KIND))? else { return Ok(None); };
        let mut superseded = message_marks::recorded_at(&self.readers.get(), subject,
            &["message.delivered", "message.read"], None)?.is_some();
        for kind in ["message.delivered", "message.read"] {
            if superseded { break; }
            if self.latest_claim(subject, Some(kind))?.is_some()
                || messages_shard::mailbox_reads::lifecycle_step(self, subject, kind)?.is_some()
            {
                superseded = true;
                break;
            }
        }
        Ok(Some(MessageExpiry { claim, superseded }))
    }
}

pub(super) fn validate_actor(kind: &str, actor: Option<&str>) -> Result<(), FractalError> {
    if kind == KIND && actor != Some("daemon/runtime") {
        return Err(FractalError::new("invalid-message-expiry", "only daemon/runtime may expire mail"));
    }
    Ok(())
}

pub(super) fn exists(connection: &Connection, subject: &str) -> Result<bool> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='message.expired')",
        [subject], |row| row.get(0))?)
}

/// Consumption supersedes expiry even when canonical field order leaves expiry last.
pub(super) fn status(connection: &Connection, subject: &str) -> Result<Option<&'static str>> {
    if !exists(connection, subject)? { return Ok(None); }
    let rank: i64 = connection.prepare_cached(
        "SELECT COALESCE(SUM(DISTINCT CASE kind WHEN 'message.delivered' THEN 1 WHEN 'message.read' THEN 2 WHEN 'message.closed' THEN 4 ELSE 0 END),0)
         FROM (SELECT kind FROM claims WHERE subject=?1
               UNION ALL SELECT kind FROM local_message_marks WHERE message=?1
               UNION ALL SELECT kind FROM local_stream_lifecycle WHERE subject=?1)",
    )?.query_row(/* sqlite-large-value: allow-for-now lv-hcb03abb7c6f1 owner=fractal_store::store::message_expiry TODO=trace-statement-and-value-representation */ [subject], |row| row.get(0))?;
    Ok(Some(if rank & 3 == 0 { "expired" } else if rank & 4 != 0 { "closed" } else if rank & 2 != 0 { "read" } else { "delivered" }))
}

/// Runtime admission holds ReceiptGate before entering the graph writer.
pub(super) fn validate_shard(runtime: &FractalRuntime, connection: &Connection, subject: &str) -> Result<(), FractalError> {
    if exists(connection, subject).map_err(internal)? { return Ok(()); }
    let slot = runtime.extensions.get::<messages_shard::Slot>();
    if let Some(shard) = slot.get() {
        let terminal = shard.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM stream_lifecycle WHERE subject=?1 AND kind IN ('message.delivered','message.read','message.closed'))",
            [subject], |row| row.get::<_, bool>(0),
        ).map_err(internal)?;
        if terminal { return Err(FractalError::new("invalid-message-transition", "shard mail already has a terminal receipt")); }
    }
    Ok(())
}

pub(super) fn validate_transition(connection: &Connection, subject: &str) -> Result<bool, FractalError> {
    if exists(connection, subject).map_err(internal)? { return Ok(true); }
    let terminal: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind IN ('message.delivered','message.read','message.closed'))
         OR EXISTS(SELECT 1 FROM local_message_marks WHERE message=?1)
         OR EXISTS(SELECT 1 FROM local_stream_lifecycle WHERE subject=?1 AND kind IN ('message.delivered','message.read','message.closed'))",
        [subject], |row| row.get(0),
    ).map_err(internal)?;
    if !message_sent::message_sent_tx(connection, subject).map_err(internal)? || terminal {
        return Err(FractalError::new("invalid-message-transition", "expiry requires sent or uncertain staged mail"));
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: &str, actor: &str) -> ClaimInput {
        ClaimInput { subject: "message/expiry-contract".into(), kind: kind.into(), actor: Some(actor.into()),
            fields: BTreeMap::from([("status".into(), json!(kind.trim_start_matches("message.")))]),
            evidence: vec![], expected_subject: None, idempotency_key: None }
    }

    fn fixture(storage: &str) -> (tempfile::TempDir, Arc<Store>, ClaimInput) {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&directory.path().join("claims.sqlite3"), "fixture").unwrap());
        let mut sent = input("message.sent", "agent/sender");
        sent.fields.extend([("from".into(), json!("agent/sender")), ("to".into(), json!("agent/fixture.recipient")),
            ("content".into(), json!("Expiry contract fixture"))]);
        if storage == "shard" {
            store.bind_fleet("expiry-fixture").unwrap();
            store.load_member_key(Some(Arc::new(smallclaims::fleet::MemberKey::generate().unwrap().0))).unwrap();
            store.use_key_directory(&directory.path().join("keys")).unwrap();
            messages_shard::open(&store, directory.path()).unwrap().append_claim(&sent).unwrap();
            stream_mirror::mirror(&store).unwrap();
        } else {
            store.append_claim(&sent).unwrap();
            if storage == "marks" {
                let intent = crate::graph::parse_intent("version 2\nagent \"recipient\" { host \"fixture\"; command \"true\"; }", "fixture").unwrap();
                crate::store::card_fold_tests_shared_missions::declare(&store, &intent, "expiry-local-seat");
                assert!(store.refresh_marks_switch(&[]).unwrap().0);
            }
        }
        let mut expiry = input(KIND, "daemon/runtime");
        expiry.fields.insert("reason".into(), json!("cutoff-uncertain-attempt"));
        (directory, store, expiry)
    }

    fn receipt(store: &Store, storage: &str, kind: &str, actor: &str) {
        let input = input(kind, actor);
        if storage == "shard" {
            messages_shard::record_lifecycle(store, &input.subject, kind, input.actor.as_deref(), &input.fields).unwrap();
            assert!(messages_shard::mailbox_reads::lifecycle_step(store, &input.subject, kind).unwrap().is_some());
        } else { store.append_claim(&input).unwrap(); }
    }

    #[test]
    fn expiry_readers_exclude_terminal_mail_and_restore_consumed_paths() {
        for storage in ["graph", "marks", "shard"] {
            let (_directory, store, expiry) = fixture(storage);
            let recipient = "agent/fixture.recipient";
            let listed = || store.messages(Some(recipient), false).unwrap();
            let summary = || store.mailbox_unread_summary(recipient, true, now_ms()).unwrap();
            let client = || {
                while store.maintain_client_message_selectors().unwrap() {}
                store.read_snapshot(|through| store.client_messages_page(Some(recipient), None, false, through, None, 20)).unwrap()
            };
            assert_eq!(listed().len(), 1);
            assert_eq!(client().len(), 1);
            store.append_claim(&expiry).unwrap();
            assert_eq!(store.message(&expiry.subject).unwrap().unwrap().status, "expired", "{storage}");
            assert!(listed().is_empty(), "{storage}");
            assert!(summary().is_empty(), "{storage}");
            assert!(client().is_empty(), "{storage}");
            if storage != "shard" {
                assert!(store.offerable_messages_for_delivery_through(recipient, store.index().unwrap()).unwrap().is_empty());
            } else {
                assert!(messages_shard::mailbox_reads::delivery_items(&store, recipient).unwrap().is_empty());
            }
            receipt(&store, storage, "message.delivered", recipient);
            stream_mirror::mirror(&store).unwrap();
            assert_eq!(store.message(&expiry.subject).unwrap().unwrap().status, "delivered");
            assert_eq!(summary().iter().map(|(_, _, count, _)| count).sum::<u64>(), 1, "{storage}");
            assert_eq!(client().len(), 1, "{storage}");
            receipt(&store, storage, "message.read", recipient);
            stream_mirror::mirror(&store).unwrap();
            assert_eq!(store.message(&expiry.subject).unwrap().unwrap().status, "read");
            assert!(summary().is_empty());
        }
    }

    #[test]
    fn expiry_contract_rejects_transactional_bypass() {
        let (_directory, store, expiry) = fixture("graph");
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let body = json!({"fields": expiry.fields});
        assert!(runtime().append_claim_tx(&transaction, "fixture", &expiry.subject, KIND,
            expiry.actor.as_deref(), &body, &[], None).is_err());
        assert!(append_claim_tx_with_assistant_answer(&transaction, "fixture", &expiry.subject, KIND,
            expiry.actor.as_deref(), &body, &[], None, None).is_err());
        assert!(!exists(&transaction, &expiry.subject).unwrap());
        transaction.commit().unwrap();
    }

    #[test]
    fn expiry_contract_read_then_delivered_is_handled() {
        for storage in ["graph", "marks", "shard"] {
            let (_directory, store, expiry) = fixture(storage);
            store.append_claim(&expiry).unwrap();
            receipt(&store, storage, "message.read", "agent/fixture.recipient");
            receipt(&store, storage, "message.delivered", "agent/fixture.recipient");
            assert!(store.message_expiry(&expiry.subject).unwrap().unwrap().superseded, "{storage}");
        }
    }

    #[test]
    fn expiry_contract_history_marks_and_late_receipts() {
        for storage in ["graph", "marks", "shard"] {
            for step in ["message.delivered", "message.read"] {
                for withdrawn in [false, true] {
                    let (directory, store, mut expiry) = fixture(storage);
                    for actor in ["agent/fixture.recipient", "daemon/other"] {
                        expiry.actor = Some(actor.into());
                        assert!(store.append_claim(&expiry).is_err());
                    }
                    expiry.actor = Some("daemon/runtime".into());
                    if withdrawn { receipt(&store, storage, "message.staged", "agent/fixture.recipient"); }
                    let claim = store.append_claim(&expiry).unwrap();
                    assert_eq!(store.append_claim(&expiry).unwrap().id, claim.id);
                    if withdrawn { receipt(&store, storage, "message.closed", "daemon/runtime"); }
                    assert!(!store.message_expiry(&expiry.subject).unwrap().unwrap().superseded);
                    receipt(&store, storage, step, "agent/fixture.recipient");
                    assert!(store.message_expiry(&expiry.subject).unwrap().unwrap().superseded, "{storage}/{step}/{withdrawn}");
                    if storage == "marks" {
                        assert!(message_marks::recorded_at(&store.readers.get(), &expiry.subject, &[step], None).unwrap().is_some());
                    }
                    drop(store);
                    let reopened = Store::open(&directory.path().join("claims.sqlite3"), "fixture").unwrap();
                    assert_eq!(reopened.message_expiry(&expiry.subject).unwrap().unwrap().claim.id, claim.id);
                }
            }
        }
    }

    #[test]
    fn expiry_contract_rejects_terminal_receipts_without_a_mirror_refresh() {
        for storage in ["graph", "marks", "shard"] {
            for terminal in ["message.delivered", "message.read", "message.closed"] {
                let (_directory, store, mut expiry) = fixture(storage);
                if terminal == "message.read" { receipt(&store, storage, "message.delivered", "agent/fixture.recipient"); }
                receipt(&store, storage, terminal, if terminal == "message.closed" { "daemon/runtime" } else { "agent/fixture.recipient" });
                assert!(store.append_claim(&expiry).is_err(), "{storage}/{terminal}");
                expiry.subject = "message/missing-expiry".into();
                assert!(store.append_claim(&expiry).is_err());
            }
        }
    }

    #[test]
    fn expiry_contract_replication_and_old_registry_are_compatible() {
        let registry = fractal_schema::registry();
        let mut old = registry.clone();
        old.claims.remove(KIND);
        assert_ne!(old.digest(), registry.digest());
        for reason in ["cutoff-never-offered", "cutoff-uncertain-attempt", "no-live-recipient"] {
            let fields = BTreeMap::from([("status".into(), json!("expired")), ("reason".into(), json!(reason))]);
            registry.validate_claim("message/expiry-contract", KIND, &fields).unwrap();
            let mut claim = messages_shard::lifecycle_receipt("fixture", "message/expiry-contract", KIND,
                Some("daemon/runtime"), &fields);
            assert!(matches!(classify_replicated_claim_with_registry(&claim, &old).unwrap(), ReplicatedClaimAdmission::UnknownKind));
            assert!(matches!(classify_replicated_claim_with_registry(&claim, registry).unwrap(), ReplicatedClaimAdmission::Valid));
            claim.actor = Some("agent/recipient".into());
            assert!(classify_replicated_claim_with_registry(&claim, registry).is_err());
            assert!(messages_shard::classify_stream_item(&claim).is_err());
        }
        let fields = BTreeMap::from([("status".into(), json!("expired")), ("reason".into(), json!("other"))]);
        assert!(registry.validate_claim("message/expiry-contract", KIND, &fields).is_err());
    }
}
