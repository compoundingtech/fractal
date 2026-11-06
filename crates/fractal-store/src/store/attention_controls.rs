//! Person-owned presentation controls. Claims, not local marks, are authoritative.
use super::*;
use std::sync::LazyLock;

pub const KIND: &str = "attention.control";
static CURRENT: LazyLock<String> = LazyLock::new(|| canonical_sql(
    "SELECT id,json_extract(body,'$.fields.source_revision')=?5,
        json_extract(body,'$.fields.operation')='dismiss',json_extract(body,'$.fields.until_unix_ms')
     FROM claims INDEXED BY claims_attention_control_index
     WHERE kind='attention.control' AND subject=?1 AND json_extract(body,'$.fields.attention_id')=?2
        AND json_extract(body,'$.fields.operation') IN (?3,?4)
        AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=claims.id)
        AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
     ORDER BY CANONICAL_DESC(claims) LIMIT 1",
));

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h0b9ffff66424 owner=fractal_store::store::attention_controls TODO=chunk-or-reference-variable-columns */ "CREATE INDEX IF NOT EXISTS claims_attention_control_index
        ON claims(subject, json_extract(body, '$.fields.attention_id'), json_extract(body, '$.fields.operation'))
        WHERE kind='attention.control';")?;
    Ok(())
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct AttentionControlState {
    pub dismissed: bool,
    pub snoozed_until_unix_ms: Option<i64>,
    pub revision: Option<String>,
}
impl AttentionControlState {
    pub fn hidden_at(&self, now: i64) -> bool {
        self.dismissed || self.snoozed_until_unix_ms.is_some_and(|until| until > now)
    }
}
#[derive(Clone, Copy, Debug)]
pub enum AttentionControlChange {
    Dismiss,
    Restore,
    Snooze(Option<i64>),
}
pub struct AttentionControlRequest<'a> {
    pub person: &'a str,
    pub attention_id: &'a str,
    pub source_revision: &'a str,
    pub expected_revision: Option<&'a str>,
    pub change: AttentionControlChange,
    pub now_unix_ms: i64,
    pub key: &'a str,
    /// Source ownership/currency is checked under the same writer lock as control CAS.
    pub validate_source: &'a ClaimAdmission<'a>,
    pub commit_receipt: Option<&'a AttentionControlCommit<'a>>,
}

/// Two indexed point reductions use the shared canonical order, never arrival order.
/// A newer source revision invalidates suppression without reviving older register values.
pub fn current(connection: &Connection, person: &str, attention: &str, source_revision: &str) -> Result<AttentionControlState> {
    smallclaims::touched::note_read(|| person.to_owned());
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h406b27c6ee25 owner=fractal_store::store::attention_controls TODO=trace-statement-and-value-representation */ &CURRENT)?;
    let mut head = |first, second| statement.query_row(
        /* sqlite-large-value: allow-for-now lv-h241dc216bf20 owner=fractal_store::store::attention_controls TODO=trace-statement-and-value-representation */ params![person, attention, first, second, source_revision],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, bool>(2)?, row.get::<_, Option<i64>>(3)?)),
    ).optional();
    let dismissed = head("dismiss", "restore")?;
    let snoozed = head("snooze", "snooze")?;
    let revision = if dismissed.is_none() && snoozed.is_none() { None } else {
        Some(canonical_hash(&(dismissed.as_ref().map(|head| &head.0), snoozed.as_ref().map(|head| &head.0)))?)
    };
    Ok(AttentionControlState {
        dismissed: dismissed.is_some_and(|(_, applies, value, _)| applies && value),
        snoozed_until_unix_ms: snoozed.and_then(|(_, applies, _, until)| applies.then_some(until).flatten()),
        revision,
    })
}

pub struct AttentionControlSource<'a> {
    pub source_id: &'a str,
    pub kind: &'a str,
    pub episode: &'a str,
}

impl Store {
    /// Rebuild the exact native source under writer admission, never through a pooled reader.
    pub fn validate_attention_control_source(
        &self, connection: &Connection, source: &AttentionControlSource<'_>,
        person: &str, attention_id: &str, revision: &str, now: u128,
    ) -> Result<(), St3Error> {
        let derive = || -> Result<Option<AttentionItemView>> {
            let item = match source.kind {
                "person-step" => attention_snapshot::person_attention_item(connection, source.source_id, now)?,
                "human-gate" | "launch-approval" | "revision-approval" =>
                    attention::mission_run_attention_item_tx(connection, source.kind, source.source_id, person, source.episode, now)?,
                "harness-prompt" => self.harness_prompt_attention_item_at(connection, source.source_id, person, source.episode, now)?,
                "harness-login" => self.harness_login_attention_item_at(connection, source.source_id, person, source.episode)?,
                "fault" => self.broken_gate_attention_item_at(connection, source.source_id, person, source.episode, now)?,
                kind if kind.starts_with("custom.") => custom::attention_item_at(connection, source.source_id)?,
                _ => None,
            };
            match item {
                Some(item) => attention::current_attention_item_tx(connection, item, now),
                None => Ok(None),
            }
        };
        let Some(item) = derive().map_err(internal)? else {
            return Err(St3Error::new("stale-fence", "attention source is no longer current"));
        };
        if item.person != person || item.kind != source.kind || item.episode != source.episode
            || crate::client_values::client_attention_id(&item.subject, &item.person, &item.episode).map_err(internal)? != attention_id
            || crate::client_values::client_attention_control_source(&item).map_err(internal)? != revision
        {
            return Err(St3Error::new("stale-fence", "attention source changed since it was read"));
        }
        Ok(())
    }

    pub fn attention_controls(&self, person: &str, attention: &str, source_revision: &str) -> Result<AttentionControlState> {
        current(&self.readers.get(), person, attention, source_revision)
    }

    pub fn write_attention_control(&self, request: &AttentionControlRequest<'_>) -> Result<ClaimRecord, St3Error> {
        let (operation, until) = match request.change {
            AttentionControlChange::Dismiss => ("dismiss", None),
            AttentionControlChange::Restore => ("restore", None),
            AttentionControlChange::Snooze(until) => ("snooze", until),
        };
        let input = ClaimInput {
            subject: request.person.into(), kind: KIND.into(), actor: Some(request.person.into()),
            fields: BTreeMap::from([
                ("attention_id".into(), json!(request.attention_id)),
                ("source_revision".into(), json!(request.source_revision)),
                ("operation".into(), json!(operation)), ("until_unix_ms".into(), json!(until)),
            ]),
            evidence: Vec::new(), expected_subject: None, idempotency_key: Some(request.key.into()),
        };
        let admission = |connection: &Connection| {
            (request.validate_source)(connection)?;
            if until.is_some_and(|until| until <= request.now_unix_ms) {
                return Err(St3Error::new("invalid-attention-snooze", "snooze deadline must be in the future"));
            }
            let controls = current(connection, request.person, request.attention_id, request.source_revision).map_err(internal)?;
            if controls.revision.as_deref() != request.expected_revision {
                return Err(St3Error::new("stale-fence", "attention controls changed since they were read"));
            }
            Ok(())
        };
        append_claim_with_commit_context(&self.graph, &input, None, None, None, None,
            ClaimCommitContext { admission: Some(&admission), attention_control_commit: request.commit_receipt, ..Default::default() }).map(|(claim, _)| claim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PERSON: &str = "person/example";
    const ITEM: &str = "attention/0123456789abcdef0123456789abcdef";
    fn write(store: &Store, change: AttentionControlChange, key: &str) -> ClaimRecord {
        let state = store.attention_controls(PERSON, ITEM, "source/one").unwrap();
        store.write_attention_control(&AttentionControlRequest {
            person: PERSON, attention_id: ITEM, source_revision: "source/one", expected_revision: state.revision.as_deref(),
            change, now_unix_ms: 100, key, validate_source: &|_| Ok(()),
            commit_receipt: None,
        }).unwrap()
    }
    #[test]
    fn independent_registers_and_explicit_clears_survive_reopen_and_replay() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("controls.sqlite");
        {
            let store = Store::open(&path, "member").unwrap();
            write(&store, AttentionControlChange::Dismiss, "dismiss-one");
            write(&store, AttentionControlChange::Snooze(Some(1000)), "snooze-one");
            write(&store, AttentionControlChange::Restore, "restore-one");
            assert!(store.attention_controls(PERSON, ITEM, "source/one").unwrap().hidden_at(999));
            assert!(!store.attention_controls(PERSON, ITEM, "source/one").unwrap().hidden_at(1000));
            write(&store, AttentionControlChange::Snooze(None), "clear-one");
            store.rebuild_claim_projections().unwrap();
        }
        let store = Store::open(&path, "member").unwrap();
        let controls = store.attention_controls(PERSON, ITEM, "source/one").unwrap();
        assert!(!controls.dismissed);
        assert_eq!(controls.snoozed_until_unix_ms, None);
        assert!(controls.revision.is_some());
        assert_eq!(store.attention_controls("person/other", ITEM, "source/one").unwrap(), AttentionControlState::default());
    }
    #[test]
    fn source_activity_wakes_controls_and_same_key_replay_precedes_admission() {
        let store = Store::open_memory("member").unwrap();
        write(&store, AttentionControlChange::Dismiss, "dismiss-one");
        assert!(!store.attention_controls(PERSON, ITEM, "source/two").unwrap().hidden_at(100));
        let first = write(&store, AttentionControlChange::Snooze(Some(1000)), "snooze-one");
        let request = AttentionControlRequest {
            person: PERSON, attention_id: ITEM, source_revision: "source/one", expected_revision: None,
            change: AttentionControlChange::Snooze(Some(1000)), now_unix_ms: 2000, key: "snooze-one",
            validate_source: &|_| Err(St3Error::new("stale-fence", "source changed")),
            commit_receipt: None,
        };
        assert_eq!(store.write_attention_control(&request).unwrap().id, first.id);
        assert_eq!(store.write_attention_control(&AttentionControlRequest { key: "new-key", ..request }).unwrap_err().code, "stale-fence");
    }
    #[test]
    fn control_cas_rejects_stale_writes_without_changing_registers() {
        let store = Store::open_memory("member").unwrap();
        write(&store, AttentionControlChange::Dismiss, "dismiss-one");
        let error = store.write_attention_control(&AttentionControlRequest {
            person: PERSON, attention_id: ITEM, source_revision: "source/one", expected_revision: None,
            change: AttentionControlChange::Restore, now_unix_ms: 100, key: "stale-restore",
            validate_source: &|_| Ok(()),
            commit_receipt: None,
        }).unwrap_err();
        assert_eq!(error.code, "stale-fence");
        assert!(store.attention_controls(PERSON, ITEM, "source/one").unwrap().dismissed);
    }

    #[test]
    fn receipt_failure_rolls_back_and_replay_keeps_original_control_revision() {
        let store = Store::open_memory("member").unwrap();
        let fail = std::sync::atomic::AtomicBool::new(true);
        let receipt_subject = "client-action/control-test";
        let observer = |transaction: &Transaction<'_>, _: &ClaimRecord, inserted: bool| {
            if !inserted { return Ok(()); }
            let controls = current(transaction, PERSON, ITEM, "source/one").map_err(internal)?;
            insert_local_observation_tx(transaction, store.origin(), &ClaimInput {
                subject: receipt_subject.into(), kind: "client.action-result".into(), actor: None,
                fields: BTreeMap::from([
                    ("authority_actor".into(), json!(PERSON)),
                    ("request_digest".into(), json!("control-test")),
                    ("result".into(), json!({"control_revision": controls.revision})),
                ]),
                evidence: vec![], expected_subject: None, idempotency_key: Some("receipt-one".into()),
            }, 100)?;
            if fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(St3Error::new("receipt-failed", "simulated interruption before commit"));
            }
            Ok(())
        };
        let request = AttentionControlRequest {
            person: PERSON, attention_id: ITEM, source_revision: "source/one", expected_revision: None,
            change: AttentionControlChange::Dismiss, now_unix_ms: 100, key: "atomic-dismiss",
            validate_source: &|_| Ok(()), commit_receipt: Some(&observer),
        };
        assert_eq!(store.write_attention_control(&request).unwrap_err().code, "receipt-failed");
        assert!(!store.attention_controls(PERSON, ITEM, "source/one").unwrap().dismissed);
        assert!(store.observations_for(receipt_subject, "client.action-result").unwrap().is_empty());
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        let accepted = store.write_attention_control(&request).unwrap();
        let original = store.observations_for(receipt_subject, "client.action-result").unwrap();
        assert_eq!(original.len(), 1);
        assert_eq!(original[0].body["fields"]["result"]["control_revision"],
            json!(store.attention_controls(PERSON, ITEM, "source/one").unwrap().revision));
        write(&store, AttentionControlChange::Restore, "intervening-restore");
        assert_eq!(store.write_attention_control(&request).unwrap().id, accepted.id);
        let replayed = store.observations_for(receipt_subject, "client.action-result").unwrap();
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].body, original[0].body);
        assert_ne!(replayed[0].body["fields"]["result"]["control_revision"],
            json!(store.attention_controls(PERSON, ITEM, "source/one").unwrap().revision));
    }
}
