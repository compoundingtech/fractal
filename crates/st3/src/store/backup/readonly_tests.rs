//! Private stage controls. None creates a copy, CLI, production fixture or live worker.
use super::*;

fn diagnostic(store: &Store, reason: &str) {
    store.append_claim(&ClaimInput {
        subject: format!("daemon/{}", store.origin()), kind: "daemon.diagnostic".into(), actor: None,
        fields: BTreeMap::from([
            ("severity".into(), json!("warning")), ("code".into(), json!("backup-read-only-test")),
            ("reason".into(), json!(reason)),
        ]), evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
}

fn sealed(path: &Path) -> Store {
    let store = Store::open(path, "alder").unwrap();
    diagnostic(&store, "retained source");
    store.seal_local_batches().unwrap();
    let connection = store.readers.get();
    assert_eq!(connection.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0)).unwrap(), CURRENT_SCHEMA_VERSION);
    let columns = connection.prepare("PRAGMA index_info(sqlite_autoindex_batches_1)").unwrap()
        .query_map([], |row| row.get::<_, String>(2)).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(columns, ["id"], "the pinned current batches primary key supplies this forced index");
    drop(connection);
    store
}

fn compare_live(store: &Store, bodies: Option<&crate::message_body::Bodies>) -> (Header, Vec<u8>) {
    let mut old = Vec::new();
    let old_header = legacy_writer_tests::write(store, &mut old, bodies).unwrap();
    let mut current = Vec::new();
    let current_header = store.write_backup_with_bodies(&mut current, bodies).unwrap();
    assert_eq!(current, old, "the pre-factoring writer is the independent byte oracle");
    assert_eq!(serde_json::to_value(&current_header).unwrap(), serde_json::to_value(&old_header).unwrap());
    (old_header, old)
}

fn compare_stage(path: &Path, old: &[u8], bodies: Option<&crate::message_body::Bodies>) -> Header {
    let before = fs::read(path).unwrap();
    let mut staged = Vec::new();
    let header = Store::write_read_only_backup(path, &mut staged, bodies).unwrap();
    assert_eq!(staged, old, "a sealed stage preserves every archive byte");
    assert_eq!(fs::read(path).unwrap(), before, "the stage main file stays unchanged");
    header
}

fn restore_pair(root: &Path, bytes: &[u8], header: &Header) {
    // Two independent restores exercise ordinary admission/projection and digest checks.
    for name in ["old", "stage"] {
        let archive = root.join(format!("{name}.jsonl"));
        fs::write(&archive, bytes).unwrap();
        backup::verify_file(&archive).unwrap();
        let report = backup::restore(&archive, &root.join(name).join("claims.sqlite3")).unwrap();
        assert!(report.projections_match);
        assert_eq!(report.graph_digest, header.graph_digest);
        assert_eq!(report.log_digest, header.log_digest);
        assert_eq!(report.tables, header.tables);
    }
}

#[test]
fn read_only_v1_matches_the_prior_writer_with_signatures_documents_and_restore() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.db");
    let store = Store::open(&path, "alder").unwrap();
    let key = Arc::new(crate::fleet::MemberKey::generate().unwrap().0);
    store.bind_fleet("test-fleet").unwrap();
    store.pin_fleet_anchor(key.public()).unwrap();
    store.set_member_key(Some(key.clone())).unwrap();
    store.admit_fleet_anchor("test-fleet", key.public(), "listening").unwrap();
    for index in 0..PAGE + 1 { diagnostic(&store, &format!("note-{index}")); }
    store.put_document("doc/read-only-backup", b"# Original bytes\n", &None, "read-only-document").unwrap();
    let (header, bytes) = compare_live(&store, None);
    assert_eq!(header.version, crate::backup::VERSION);
    assert!(String::from_utf8_lossy(&bytes).contains("\"record\":\"signatures\""));
    drop(store);
    compare_stage(&path, &bytes, None);
    restore_pair(root.path(), &bytes, &header);
}

#[test]
fn read_only_v2_preserves_owned_missing_mismatched_and_stray_body_semantics() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("source");
    fs::create_dir_all(&state).unwrap();
    let path = state.join("claims.sqlite3");
    let store = sealed(&path);
    let bodies = crate::message_body::bodies(&state);
    let body = "a long body with UTF-8 é\n".repeat(500);
    for id in ["owned", "missing", "mismatch", "foreign-holder"] {
        let sender = if id == "foreign-holder" { "person/blair" } else { "person/alex" };
        store.append_claim(&ClaimInput {
            subject: format!("message/{id}"), kind: "message.sent".into(),
            actor: Some(sender.into()),
            fields: BTreeMap::from([
                ("from".into(), json!(sender)), ("to".into(), json!("agent/reader")),
                ("content".into(), json!(crate::message_body::preview(&body))),
                ("status".into(), json!("sent")),
                ("attachments".into(), json!([{
                    "sha256": hex::encode(Sha256::digest(body.as_bytes())),
                    "media_type": "text/plain", "size": body.len(), "origin": "host/alder"
                }])),
            ]), evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
    }
    bodies.put("person/alex", "message/owned", body.as_bytes()).unwrap();
    bodies.put("person/alex", "message/mismatch", b"different bytes").unwrap();
    bodies.put("person/alex", "message/stray", b"no claim names this").unwrap();
    bodies.put("person/alex", "message/foreign-holder", body.as_bytes()).unwrap();
    let before_receipt = store.message("message/owned").unwrap().unwrap();
    store.append_claim(&ClaimInput {
        subject: "message/owned".into(), kind: "message.read".into(), actor: Some("agent/reader".into()),
        fields: BTreeMap::from([("status".into(), json!("read"))]),
        evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    let after_receipt = store.message("message/owned").unwrap().unwrap();
    assert_eq!(after_receipt.created_index, before_receipt.created_index);
    assert_eq!(after_receipt.status, "read", "creation index is display identity, not an as-of cutoff");
    assert_eq!(after_receipt.from, "person/alex");
    assert_eq!(after_receipt.body_ref, before_receipt.body_ref);
    let (header, bytes) = compare_live(&store, Some(&bodies));
    assert_eq!(header.version, crate::backup::VERSION_WITH_BODIES);
    assert_eq!(String::from_utf8_lossy(&bytes).matches("\"record\":\"message-body\"").count(), 1);
    drop(store);
    compare_stage(&path, &bytes, Some(&bodies));
    restore_pair(root.path(), &bytes, &header);
    for name in ["old", "stage"] {
        let restored = crate::message_body::bodies(&root.path().join(name));
        assert_eq!(restored.read("person/alex", "message/owned").unwrap().unwrap(), body.as_bytes());
        assert!(!restored.exists("person/alex", "message/missing"));
        assert!(!restored.exists("person/alex", "message/mismatch"));
        assert!(!restored.exists("person/alex", "message/stray"));
        assert!(!restored.exists("person/alex", "message/foreign-holder"));
        assert!(!restored.exists("person/blair", "message/foreign-holder"));
    }
}

#[test]
fn read_only_certified_and_interrupted_trim_match_prior_bytes_and_restore() {
    for interrupted in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.db");
        let store = sealed(&path);
        store.bind_fleet("test-fleet").unwrap();
        for index in 0..12 { diagnostic(&store, &format!("note-{index}")); }
        let context = CheckpointContext {
            now_unix_ms: now_ms() + 3 * 86_400_000, configured_peers: vec![],
            scratch: root.path().join("scratch"), reviewer: "person/operator".into(),
        };
        for _ in 0..2 { store.checkpoint_step(&context).unwrap(); }
        if interrupted {
            store.set_trim_fault(Some(TrimFault::AfterChunk(1)));
            assert!(store.checkpoint_step(&context).is_err());
            assert!(store.trimmed_checkpoint().unwrap().is_none());
        } else {
            store.checkpoint_step(&context).unwrap();
            assert!(store.trimmed_checkpoint().unwrap().is_some());
        }
        assert!(store.checkpointed_envelopes().unwrap() > 0);
        let (header, bytes) = compare_live(&store, None);
        assert!(String::from_utf8_lossy(&bytes).contains("\"record\":\"checkpoint\""));
        drop(store);
        compare_stage(&path, &bytes, None);
        restore_pair(root.path(), &bytes, &header);
    }
}

#[test]
fn read_only_refuses_unsealed_and_skipped_below_cursor_claims_without_writes() {
    for skipped in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.db");
        let store = sealed(&path);
        diagnostic(&store, "not yet sealed");
        if skipped {
            // Reader/storage conformance: model the known stale cursor, without deleting
            // any source/envelope, disabling indexes or invoking recovery/sealing.
            let connection = store.connection.write();
            let high = max_batch_rowid(&connection).unwrap();
            connection.execute("UPDATE meta SET value=?1 WHERE key='seeded_batch_rowid'", [high.to_string()]).unwrap();
        }
        drop(store);
        let before = fs::read(&path).unwrap();
        let mut bytes = Vec::new();
        let error = Store::write_read_only_backup(&path, &mut bytes, None).unwrap_err();
        assert!(error.to_string().contains(if skipped { "without an envelope" } else { "unsealed batch frontier" }), "{error:#}");
        assert!(bytes.is_empty());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn read_only_refuses_old_or_incomplete_layout_and_unprojected_cut_without_repair() {
    for case in ["old-version", "missing-registry", "missing-digest", "unprojected"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.db");
        let store = sealed(&path);
        {
            let connection = store.connection.write();
            match case {
                "old-version" => connection.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION - 1).unwrap(),
                "missing-registry" => { connection.execute("DELETE FROM meta WHERE key='projection_digest_registry'", []).unwrap(); }
                "missing-digest" => { connection.execute("DELETE FROM projection_digest_state WHERE table_name='claim_sources'", []).unwrap(); }
                _ => { connection.execute("UPDATE projection_health SET status='stale' WHERE aggregate='graph'", []).unwrap(); }
            }
        }
        drop(store);
        let before = fs::read(&path).unwrap();
        let mut bytes = Vec::new();
        let error = Store::write_read_only_backup(&path, &mut bytes, None).unwrap_err();
        if case == "unprojected" {
            assert!(error.to_string().contains("fully projected graph"), "{error:#}");
            assert!(error.to_string().contains("retry after replication catches up"));
            assert!(!error.to_string().contains("predecessor"), "projection lag is not a chain gap");
        }
        assert!(bytes.is_empty(), "{case}");
        assert_eq!(fs::read(&path).unwrap(), before, "{case}");
    }
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("absent.db");
    assert!(Store::write_read_only_backup(&missing, &mut Vec::new(), None).is_err());
    assert!(!missing.exists(), "read-only open never creates a stage");
}

#[test]
fn read_only_sink_error_ends_the_actual_snapshot_and_preserves_the_stage() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.db");
    drop(sealed(&path));
    let before = fs::read(&path).unwrap();
    let connection = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let denied = Arc::new(AtomicUsize::new(0));
    let observed = denied.clone();
    connection.authorizer(Some(move |context: AuthContext<'_>| {
        match context.action {
            AuthAction::Insert { .. } | AuthAction::Update { .. } | AuthAction::Delete { .. } |
            AuthAction::CreateTable { .. } | AuthAction::CreateIndex { .. } |
            AuthAction::CreateTrigger { .. } | AuthAction::DropTable { .. } |
            AuthAction::DropIndex { .. } | AuthAction::DropTrigger { .. } => {
                observed.fetch_add(1, Ordering::Relaxed);
                Authorization::Deny
            }
            _ => Authorization::Allow,
        }
    }));
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stage sink closed"))
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let error = source::write_read_only_connection(&connection, &mut Broken, None).unwrap_err();
    assert!(error.chain().any(|cause| cause.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe)));
    assert!(connection.is_autocommit(), "the real read transaction ended on the sink error");
    assert_eq!(connection.total_changes(), 0);
    assert_eq!(denied.load(Ordering::Relaxed), 0, "no attempted DDL/DML was hidden by read-only mode");
    let mut good = Vec::new();
    source::write_read_only_connection(&connection, &mut good, None).unwrap();
    assert!(connection.is_autocommit());
    assert_eq!(connection.total_changes(), 0);
    connection.close().unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    let archive = root.path().join("complete.jsonl");
    fs::write(&archive, good).unwrap();
    backup::verify_file(&archive).unwrap();
}

#[test]
fn a_body_changed_after_header_is_omitted_by_both_writers_and_reported_on_restore() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("source");
    fs::create_dir_all(&state).unwrap();
    let path = state.join("claims.sqlite3");
    let store = sealed(&path);
    let bodies = crate::message_body::bodies(&state);
    let body = "original long message body\n".repeat(500);
    store.append_claim(&ClaimInput {
        subject: "message/changed-body".into(), kind: "message.sent".into(),
        actor: Some("person/alex".into()),
        fields: BTreeMap::from([
            ("from".into(), json!("person/alex")), ("to".into(), json!("agent/reader")),
            ("content".into(), json!(crate::message_body::preview(&body))),
            ("status".into(), json!("sent")),
            ("attachments".into(), json!([{
                "sha256": hex::encode(Sha256::digest(body.as_bytes())),
                "media_type": "text/plain", "size": body.len(), "origin": "host/alder"
            }])),
        ]), evidence: vec![], expected_subject: None, idempotency_key: None,
    }).unwrap();
    struct ChangeAfterHeader<'a> { bytes: Vec<u8>, bodies: &'a crate::message_body::Bodies }
    impl Write for ChangeAfterHeader<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.bytes.is_empty() {
                self.bodies.put("person/alex", "message/changed-body", b"changed after the database cut")?;
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    bodies.put("person/alex", "message/changed-body", body.as_bytes()).unwrap();
    let mut old = ChangeAfterHeader { bytes: vec![], bodies: &bodies };
    let header = legacy_writer_tests::write(&store, &mut old, Some(&bodies)).unwrap();
    assert_eq!(header.version, crate::backup::VERSION_WITH_BODIES);
    assert!(!String::from_utf8_lossy(&old.bytes).contains("\"record\":\"message-body\""));
    drop(store);
    bodies.put("person/alex", "message/changed-body", body.as_bytes()).unwrap();
    let mut staged = ChangeAfterHeader { bytes: vec![], bodies: &bodies };
    Store::write_read_only_backup(&path, &mut staged, Some(&bodies)).unwrap();
    assert_eq!(staged.bytes, old.bytes);
    let archive = root.path().join("changed-body.jsonl");
    fs::write(&archive, staged.bytes).unwrap();
    let report = backup::restore(&archive, &root.path().join("restored/claims.sqlite3")).unwrap();
    assert!(report.projections_match);
    assert_eq!(report.message_bodies_referenced, 1);
    assert_eq!(report.message_bodies_restored, 0);
    assert_eq!(report.message_bodies_unmatched, 0);
}
