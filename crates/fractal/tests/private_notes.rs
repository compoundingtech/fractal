use fractal_daemon::private_notes::{Authority, NotesWrite, WriteContext};
use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};

struct Fixture {
    root: tempfile::TempDir,
    authority: Authority,
    uri: String,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let catalog = root.path().join("catalog");
        let subject = catalog.join("agents/node/worker");
        fs::create_dir_all(subject.join("resources")).unwrap();
        let uri = "dev.schickling.agent-private-notes://node/worker".to_owned();
        fs::write(subject.join("agent.kdl"), format!("agent \"worker\" {{\n host \"node\"\n resource \"notes\" uri=\"{uri}\"\n}}\n")).unwrap();
        Self { root, authority: Authority { catalogs: vec![catalog] }, uri }
    }
    fn carrier(&self) -> std::path::PathBuf { self.authority.catalogs[0].join("agents/node/worker/resources/private-notes.md") }
    fn write(&self, key: &str, write: &NotesWrite) -> Result<serde_json::Value, fractal_daemon::model::St3Error> {
        self.authority.write("node", self.root.path(), "person/operator", key, write)
    }
}

#[test]
fn competing_edits_refuse_stale_fences_and_exact_retries_do_not_overwrite_newer_content() {
    let fixture = Fixture::new();
    let empty = fixture.authority.read("node", &fixture.uri).unwrap();
    assert_eq!(empty.markdown, "");
    let first = NotesWrite { uri: fixture.uri.clone(), markdown: "owner first\n".into(), fence: empty.fence.clone() };
    let first_receipt = fixture.write("first", &first).unwrap();
    let stale = NotesWrite { uri: fixture.uri.clone(), markdown: "concurrent editor\n".into(), fence: empty.fence };
    assert_eq!(fixture.write("concurrent", &stale).unwrap_err().code, "stale-fence");
    let current = fixture.authority.read("node", &fixture.uri).unwrap();
    let newer = NotesWrite { uri: fixture.uri.clone(), markdown: "owner newer\n".into(), fence: current.fence };
    fixture.write("newer", &newer).unwrap();
    assert_eq!(fixture.write("first", &first).unwrap(), first_receipt);
    assert_eq!(fs::read_to_string(fixture.carrier()).unwrap(), "owner newer\n");
    let changed = NotesWrite { markdown: "reused key\n".into(), ..first };
    assert_eq!(fixture.write("first", &changed).unwrap_err().code, "idempotency-conflict");
}

#[test]
fn replacing_the_carrier_directory_invalidates_even_an_exact_retry() {
    let fixture = Fixture::new();
    let notes = fixture.authority.read("node", &fixture.uri).unwrap();
    let write = NotesWrite { uri: fixture.uri.clone(), markdown: "private\n".into(), fence: notes.fence };
    fixture.write("first", &write).unwrap();
    let directory = fixture.carrier().parent().unwrap().to_owned();
    fs::rename(&directory, directory.with_extension("old")).unwrap();
    fs::create_dir(&directory).unwrap();
    assert_eq!(fixture.write("first", &write).unwrap_err().code, "stale-fence");
    assert!(!fixture.carrier().exists());
}

#[test]
fn symlink_and_shared_write_permission_never_supply_or_replace_private_bytes() {
    let fixture = Fixture::new();
    let outside = fixture.root.path().join("outside.md");
    fs::write(&outside, "not notes\n").unwrap();
    symlink(&outside, fixture.carrier()).unwrap();
    assert!(fixture.authority.read("node", &fixture.uri).is_err());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "not notes\n");
    fs::remove_file(fixture.carrier()).unwrap();
    fs::write(fixture.carrier(), "private\n").unwrap();
    fs::set_permissions(fixture.carrier(), fs::Permissions::from_mode(0o660)).unwrap();
    assert_eq!(fixture.authority.read("node", &fixture.uri).unwrap_err().code, "forbidden");
    assert_eq!(fixture.authority.read("other-node", &fixture.uri).unwrap_err().code, "forbidden");
}

#[test]
fn simultaneous_editors_have_one_winner_and_the_loser_cannot_replace_it() {
    let fixture = Fixture::new();
    let initial = fixture.authority.read("node", &fixture.uri).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let outcomes = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            fixture.write("editor-one", &NotesWrite { uri: fixture.uri.clone(), markdown: "one\n".into(), fence: initial.fence.clone() })
        });
        let second = scope.spawn(|| {
            barrier.wait();
            fixture.write("editor-two", &NotesWrite { uri: fixture.uri.clone(), markdown: "two\n".into(), fence: initial.fence.clone() })
        });
        [first.join().unwrap(), second.join().unwrap()]
    });
    match (&outcomes[0], &outcomes[1]) {
        (Ok(receipt), Err(error)) => {
            assert_eq!(error.code, "stale-fence");
            assert_eq!(fs::read_to_string(fixture.carrier()).unwrap(), "one\n");
            assert_eq!(receipt["private_notes"]["revision"], fixture.authority.read("node", &fixture.uri).unwrap().fence.revision);
        }
        (Err(error), Ok(receipt)) => {
            assert_eq!(error.code, "stale-fence");
            assert_eq!(fs::read_to_string(fixture.carrier()).unwrap(), "two\n");
            assert_eq!(receipt["private_notes"]["revision"], fixture.authority.read("node", &fixture.uri).unwrap().fence.revision);
        }
        other => panic!("exactly one editor must complete: {other:?}"),
    }
    assert_eq!(fs::metadata(fixture.carrier()).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn encoded_response_limit_refuses_unreadable_writes_without_changing_the_carrier() {
    let fixture = Fixture::new();
    let empty = fixture.authority.read("node", &fixture.uri).unwrap();
    let allowed = NotesWrite { uri: fixture.uri.clone(), markdown: "\n".repeat((1_048_576 - 16_384) / 2), fence: empty.fence };
    fixture.write("at-boundary", &allowed).unwrap();
    let current = fixture.authority.read("node", &fixture.uri).unwrap();
    assert_eq!(current.markdown, allowed.markdown);
    let oversized = NotesWrite { markdown: format!("{}\n", allowed.markdown), fence: current.fence, ..allowed };
    assert_eq!(fixture.write("over-boundary", &oversized).unwrap_err().code, "validation-failed");
    assert_eq!(fs::read_to_string(fixture.carrier()).unwrap(), oversized.markdown[..oversized.markdown.len() - 1]);
}

#[test]
fn completed_receipt_remains_recoverable_while_a_later_replacement_is_pending() {
    let fixture = Fixture::new();
    let initial = fixture.authority.read("node", &fixture.uri).unwrap();
    let first = NotesWrite { uri: fixture.uri.clone(), markdown: "completed first\n".into(), fence: initial.fence };
    let receipt = fixture.write("first", &first).unwrap();
    let later = NotesWrite { uri: fixture.uri.clone(), markdown: "later replacement\n".into(), fence: fixture.authority.read("node", &fixture.uri).unwrap().fence };
    let ledger = rusqlite::Connection::open(fixture.root.path().join("private-notes.sqlite")).unwrap();
    ledger.execute_batch("CREATE TRIGGER fail_completion BEFORE UPDATE OF result ON notes_operations BEGIN SELECT RAISE(FAIL, 'injected completion failure'); END;").unwrap();
    assert_eq!(fixture.write("later", &later).unwrap_err().code, "private-notes-unreachable");
    assert_eq!(fixture.write("first", &first).unwrap(), receipt);
    assert_eq!(fs::read_to_string(fixture.carrier()).unwrap(), later.markdown);
    let before_recovery = fs::metadata(fixture.carrier()).unwrap();
    ledger.execute_batch("DROP TRIGGER fail_completion;").unwrap();
    fixture.write("later", &later).unwrap();
    use std::os::unix::fs::MetadataExt as _;
    assert_eq!(fs::metadata(fixture.carrier()).unwrap().ino(), before_recovery.ino());
}


#[test]
fn requested_audit_refusal_never_changes_private_bytes() {
    let fixture = Fixture::new();
    let initial = fixture.authority.read("node", &fixture.uri).unwrap();
    let write = NotesWrite { uri: fixture.uri.clone(), markdown: "not admitted\n".into(), fence: initial.fence };
    let result = fixture.authority.write_with_audit(
        WriteContext { node: "node", state_dir: fixture.root.path(), actor: "person/operator", key: "refused" }, &write,
        || Err(fractal_daemon::model::St3Error::new("forbidden", "injected requested audit refusal")),
        || panic!("completion cannot run without requested audit"),
    );
    assert_eq!(result.unwrap_err().code, "forbidden");
    assert!(!fixture.carrier().exists());
    fixture.write("refused", &write).unwrap();
}

#[test]
fn completion_audit_failure_survives_restart_and_requires_exact_key_recovery() {
    use std::os::unix::fs::MetadataExt as _;
    let fixture = Fixture::new();
    let initial = fixture.authority.read("node", &fixture.uri).unwrap();
    let write = NotesWrite { uri: fixture.uri.clone(), markdown: "durable private replacement\n".into(), fence: initial.fence };
    let result = fixture.authority.write_with_audit(
        WriteContext { node: "node", state_dir: fixture.root.path(), actor: "person/operator", key: "original" }, &write, || Ok(()),
        || Err(fractal_daemon::model::St3Error::new("internal", "injected completion audit refusal")),
    );
    assert_eq!(result.unwrap_err().code, "private-notes-indeterminate");
    let before = fs::metadata(fixture.carrier()).unwrap();
    let restarted = Authority { catalogs: fixture.authority.catalogs.clone() };
    let later = NotesWrite { uri: fixture.uri.clone(), markdown: "must wait\n".into(), fence: restarted.read("node", &fixture.uri).unwrap().fence };
    assert_eq!(restarted.write("node", fixture.root.path(), "person/operator", "later", &later).unwrap_err().code, "private-notes-indeterminate");
    assert_eq!(restarted.write("node", fixture.root.path(), "person/operator-two", "original", &write).unwrap_err().code, "private-notes-indeterminate");
    let changed = NotesWrite { markdown: "different request\n".into(), ..write.clone() };
    assert_eq!(fixture.write("original", &changed).unwrap_err().code, "idempotency-conflict");
    restarted.write_with_audit(WriteContext { node: "node", state_dir: fixture.root.path(), actor: "person/operator", key: "original" }, &write, || Ok(()), || Ok(())).unwrap();
    let after = fs::metadata(fixture.carrier()).unwrap();
    assert_eq!((before.ino(), before.mtime(), before.mtime_nsec()), (after.ino(), after.mtime(), after.mtime_nsec()));
    assert_eq!(fs::read_to_string(fixture.carrier()).unwrap(), write.markdown);
    fixture.write("later", &later).unwrap();
}

#[test]
fn symlinked_catalog_components_and_unsafe_subjects_never_select_a_carrier() {
    let fixture = Fixture::new();
    let resources = fixture.carrier().parent().unwrap().to_owned();
    let outside = fixture.root.path().join("outside");
    fs::rename(&resources, &outside).unwrap();
    symlink(&outside, &resources).unwrap();
    assert!(fixture.authority.read("node", &fixture.uri).is_err());
    for uri in [
        "dev.schickling.agent-private-notes://node/%2E%2E",
        "dev.schickling.agent-private-notes://node/%2Foutside",
        "dev.schickling.agent-private-notes://node/worker/extra",
    ] {
        assert_eq!(fixture.authority.read("node", uri).unwrap_err().code, "validation-failed");
    }
    assert!(!outside.join("private-notes.md").exists());
}
